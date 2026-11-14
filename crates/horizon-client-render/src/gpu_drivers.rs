//! Custom GPU drivers on Android (arm64-v8a): Turnip, PanVK, RADV for Xclipse and other
//! Vulkan drivers shipped as `.adpkg`/`.zip` packages, loaded through libadrenotools.
//!
//! A package holds `meta.json` (the AdrenoTools/Skyline schema: `name`, `libraryName`, ...)
//! and the driver `.so` files. Packages live in `gpu-drivers/` of the app's own files
//! (`HORIZON_PRIVATE`), never on shared storage: Android refuses `dlopen` of libraries
//! another user could change, and adrenotools requires it. The game never executes them;
//! `android.rs` hands the chosen folder to `adrenotools_open_libvulkan` before the renderer
//! starts, at the next start after the player picks one.
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// The package manifest, at the archive's root (or one folder down).
pub const META: &str = "meta.json";
/// The chosen driver: the `gpu-drivers/` folder's name, or missing for the system driver.
const CURRENT: &str = "current";
/// Starts with the chosen driver that ended before the first frame.
const STARTS: &str = "starts";
/// Why the chosen driver was set aside, shown once.
const NOTICE: &str = "notice";
/// Where a custom driver's file accesses are redirected to
/// (`ADRENOTOOLS_DRIVER_FILE_REDIRECT`).
pub const REDIRECT: &str = "redirect";
/// Starts that ended before the first frame, after which the driver is set aside and the
/// system driver plays instead (a native mod's build is set aside after two as well).
pub const TRIES: u32 = 2;

/// The driver loaded this process (`set_active`, Android only): its folder id and the
/// `vkGetInstanceProcAddr` its libvulkan handle gave, for the renderer's Vulkan instance.
static ACTIVE: std::sync::OnceLock<(String, usize)> = std::sync::OnceLock::new();

/// Remember the custom driver this process plays with. Called once, before the renderer
/// starts; later calls keep the first.
pub fn set_active(id: &str, get_instance_proc_addr: usize) {
    let _ = ACTIVE.set((id.to_owned(), get_instance_proc_addr));
}

/// The custom driver this process plays with, if any: `(folder id, vkGetInstanceProcAddr)`.
pub fn active() -> Option<(&'static str, usize)> {
    ACTIVE.get().map(|(id, addr)| (id.as_str(), *addr))
}

/// A driver package's `meta.json` (the AdrenoTools/Skyline `meta.json` schema, version 1).
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct Metadata {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default, rename = "packageVersion")]
    pub package_version: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default, rename = "driverVersion")]
    pub driver_version: String,
    /// The lowest Android API the package supports (`minApi`).
    #[serde(default, rename = "minApi")]
    pub min_api: i32,
    /// The driver library to load (`libraryName`): a file name, loaded from the folder.
    #[serde(default, rename = "libraryName")]
    pub library_name: String,
}

/// An installed driver: its folder's name and manifest.
#[derive(Clone, Debug)]
pub struct Driver {
    pub id: String,
    pub meta: Metadata,
}

impl Driver {
    /// The folder holding the driver's libraries.
    pub fn dir(&self) -> PathBuf {
        dir().join(&self.id)
    }

    /// The library adrenotools loads.
    pub fn library(&self) -> PathBuf {
        self.dir().join(&self.meta.library_name)
    }

    /// What the list shows: the name and the driver version.
    pub fn label(&self) -> String {
        if self.meta.driver_version.is_empty() {
            self.meta.name.clone()
        } else {
            format!("{} ({})", self.meta.name, self.meta.driver_version)
        }
    }
}

/// Where drivers are installed: the app's own files, never shared storage.
pub fn dir() -> PathBuf {
    crate::logging::private_home().join("gpu-drivers")
}

/// The installed drivers, by name.
pub fn list() -> Vec<Driver> {
    list_in(&dir())
}

fn list_in(dest: &Path) -> Vec<Driver> {
    let mut drivers = Vec::new();
    let Ok(entries) = std::fs::read_dir(dest) else {
        return drivers;
    };
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        let folder = dest.join(&id);
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(folder.join(META)) else {
            continue;
        };
        let Ok(meta): Result<Metadata, _> = serde_json::from_str(&text) else {
            continue;
        };
        if !validate_library_name(&meta.library_name) || !folder.join(&meta.library_name).is_file()
        {
            continue;
        }
        drivers.push(Driver { id, meta });
    }
    drivers.sort_by(|a, b| a.meta.name.cmp(&b.meta.name).then(a.id.cmp(&b.id)));
    drivers
}

/// The chosen driver's folder name, if any and installed.
pub fn selected() -> Option<String> {
    selected_in(&dir())
}

fn selected_in(dest: &Path) -> Option<String> {
    let id = std::fs::read_to_string(dest.join(CURRENT))
        .ok()?
        .trim()
        .to_owned();
    if id.is_empty() || !dest.join(&id).is_dir() {
        return None;
    }
    list_in(dest).into_iter().find(|d| d.id == id).map(|d| d.id)
}

/// Choose the driver with this folder name for the next start; empty chooses the system driver.
pub fn select(id: &str) {
    select_in(id, &dir());
}

fn select_in(id: &str, dest: &Path) {
    let _ = std::fs::create_dir_all(dest);
    let _ = std::fs::write(dest.join(CURRENT), id);
}

/// Delete the driver with this folder name; the system driver plays next if it was chosen.
pub fn remove(id: &str) {
    let safe = !id.is_empty() && !id.contains(['/', '\\']) && id != "." && id != "..";
    if safe {
        let _ = std::fs::remove_dir_all(dir().join(id));
    }
    if selected().is_none() {
        let _ = std::fs::remove_file(dir().join(CURRENT));
    }
}

/// Whether `name` is a loadable library file name: a plain `.so` file, no folders.
pub fn validate_library_name(name: &str) -> bool {
    !name.is_empty()
        && name.ends_with(".so")
        && !name.contains(['/', '\\'])
        && Path::new(name).file_name().and_then(|f| f.to_str()) == Some(name)
}

/// Whether these first bytes are an AArch64 ELF library.
pub fn is_elf_aarch64(head: &[u8]) -> bool {
    head.len() >= 20
        && head[0..4] == [0x7f, b'E', b'L', b'F']
        && head[4] == 2 // 64-bit
        && u16::from_le_bytes([head[18], head[19]]) == 183 // EM_AARCH64
}

/// Whether the device runs this driver: its `minApi` or older. (`meta.json` carries no
/// machine-readable GPU id, only display names, so generation matching is left to the
/// package author and the player; a driver for another GPU fails back to the system one.)
pub fn compatible(meta: &Metadata, device_api: i32) -> bool {
    device_api >= meta.min_api
}

/// The start may use the chosen driver: counts this start, and sets the driver aside with a
/// notice once `TRIES` starts in a row ended before the first frame. True for the system driver.
pub fn note_start() -> bool {
    note_start_in(&dir())
}

fn note_start_in(dest: &Path) -> bool {
    let Some(id) = selected_in(dest) else {
        return true;
    };
    let _ = std::fs::create_dir_all(dest);
    let count = std::fs::read_to_string(dest.join(STARTS))
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok())
        .unwrap_or(0);
    if count >= TRIES
        && let Some(driver) = list_in(dest).into_iter().find(|d| d.id == id)
    {
        let _ = std::fs::remove_file(dest.join(CURRENT));
        let _ = std::fs::remove_file(dest.join(STARTS));
        let _ = std::fs::write(
            dest.join(NOTICE),
            format!(
                "The {} driver closed {} times before it showed, so Starling started with the system driver.",
                driver.label(),
                TRIES,
            ),
        );
        return false;
    }
    let _ = std::fs::write(dest.join(STARTS), (count + 1).to_string());
    true
}

/// The game showed its first frame: the chosen driver started.
pub fn note_first_frame() {
    note_first_frame_in(&dir());
}

fn note_first_frame_in(dest: &Path) {
    let _ = std::fs::remove_file(dest.join(STARTS));
}

/// Why the chosen driver was set aside, once.
pub fn take_notice() -> Option<String> {
    take_notice_in(&dir())
}

fn take_notice_in(dest: &Path) -> Option<String> {
    let path = dest.join(NOTICE);
    let notice = std::fs::read_to_string(&path)
        .ok()
        .filter(|n| !n.trim().is_empty())?;
    let _ = std::fs::remove_file(path);
    Some(notice)
}

/// Install the package at `path`: `meta.json` (at the root or one folder down) names the
/// driver library, which must be an AArch64 ELF library in the same place. Every file is
/// extracted flat (by file name, so hostile paths cannot escape), into a new folder of
/// `gpu-drivers/` named after `title`.
pub fn install_from_zip(path: &Path, title: &str) -> Result<Driver, String> {
    // The hook libraries ship in the arm64-v8a APK only: anywhere else a driver could
    // never load, so say so instead of installing files the game would ignore.
    if !(cfg!(target_os = "android") && cfg!(target_arch = "aarch64")) {
        return Err("custom GPU drivers need Starling on an ARM64 phone".to_owned());
    }
    install_into(path, title, &dir())
}

/// The most a package may unpack: drivers are tens of megabytes; this stops archive bombs.
const MAX_EXTRACT_BYTES: u64 = 256 << 20;

fn install_into(path: &Path, title: &str, dest: &Path) -> Result<Driver, String> {
    let mut archive = zip::ZipArchive::new(
        File::open(path).map_err(|e| format!("could not read {}: {e}", path.display()))?,
    )
    .map_err(|_| format!("{} is not a driver package", path.display()))?;
    let names: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok()?.enclosed_name())
        .filter_map(|p| p.to_str().map(str::to_owned))
        .collect();
    let prefix = names
        .iter()
        .any(|n| n == META)
        .then(String::new)
        .or_else(|| {
            names.iter().find_map(|n| {
                let (top, rest) = n.split_once('/')?;
                (rest == META).then(|| format!("{top}/"))
            })
        })
        .ok_or_else(|| format!("{} holds no driver (no {})", path.display(), META))?;
    let manifest: Metadata = {
        let mut entry = archive
            .by_name(&format!("{prefix}{META}"))
            .map_err(|_| format!("{} holds no driver (no {})", path.display(), META))?;
        let mut text = String::new();
        entry
            .read_to_string(&mut text)
            .map_err(|e| format!("{}'s {META} cannot be read: {e}", path.display()))?;
        serde_json::from_str(&text)
            .map_err(|e| format!("{}'s {META} is not a driver manifest: {e}", path.display()))?
    };
    if manifest.name.trim().is_empty() {
        return Err(format!("{}'s {META} names no driver", path.display()));
    }
    if !validate_library_name(&manifest.library_name) {
        return Err(format!(
            "{}'s {META} names no driver library ({})",
            path.display(),
            manifest.library_name
        ));
    }
    let library = format!("{prefix}{}", manifest.library_name);
    if !names.iter().any(|n| n == &library) {
        return Err(format!(
            "{} holds no {}",
            path.display(),
            manifest.library_name
        ));
    }
    {
        let mut entry = archive
            .by_name(&library)
            .map_err(|_| format!("{} holds no {}", path.display(), manifest.library_name))?;
        let mut head = [0u8; 20];
        entry.read_exact(&mut head).map_err(|_| {
            format!(
                "{}'s {} cannot be read",
                path.display(),
                manifest.library_name
            )
        })?;
        if !is_elf_aarch64(&head) {
            return Err(format!(
                "{}'s {} is not an ARM64 driver library",
                path.display(),
                manifest.library_name
            ));
        }
    }
    let id = unique_title_in(title, dest);
    let to = dest.join(&id);
    std::fs::create_dir_all(&to).map_err(|e| format!("could not add the driver: {e}"))?;
    let mut installed = 0;
    let mut seen = std::collections::HashSet::new();
    let mut remaining = MAX_EXTRACT_BYTES;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| {
            let _ = std::fs::remove_dir_all(&to);
            format!("{} cannot be read: {e}", path.display())
        })?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let Ok(rel) = name.strip_prefix(prefix.trim_end_matches('/')) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        // Flat, by file name: the hook loads `folder/libraryName`. A second file under
        // the same name (another folder's library over the validated one) is rejected.
        let Some(base) = rel.file_name().map(|b| b.to_string_lossy().into_owned()) else {
            continue;
        };
        if base.is_empty() || base != base.trim() || base.contains(['/', '\\']) {
            continue;
        }
        if !seen.insert(base.clone()) {
            let _ = std::fs::remove_dir_all(&to);
            return Err(format!("{} holds {base} twice", path.display()));
        }
        if entry.size() > remaining {
            let _ = std::fs::remove_dir_all(&to);
            return Err(format!("{} is too large to be a driver", path.display()));
        }
        let mut out = File::create(to.join(&base)).map_err(|e| {
            let _ = std::fs::remove_dir_all(&to);
            format!("could not add the driver: {e}")
        })?;
        let written = io::copy(&mut entry, &mut out).map_err(|e| {
            let _ = std::fs::remove_dir_all(&to);
            format!("could not add the driver: {e}")
        })?;
        remaining = remaining.checked_sub(written).ok_or_else(|| {
            let _ = std::fs::remove_dir_all(&to);
            format!("{} is too large to be a driver", path.display())
        })?;
        installed += 1;
    }
    if installed == 0 || !to.join(&manifest.library_name).is_file() {
        let _ = std::fs::remove_dir_all(&to);
        return Err(format!("{} holds no driver files", path.display()));
    }
    // The library on disk, as the hook will load it: what was validated above may have
    // been a different file under the same name.
    let library = std::fs::read(to.join(&manifest.library_name)).map_err(|e| {
        let _ = std::fs::remove_dir_all(&to);
        format!("could not add the driver: {e}")
    })?;
    if library.len() < 20 || !is_elf_aarch64(&library[..20]) {
        let _ = std::fs::remove_dir_all(&to);
        return Err(format!(
            "{}'s {} is not an ARM64 driver library",
            path.display(),
            manifest.library_name
        ));
    }
    // The manifest the list reads.
    std::fs::create_dir_all(to.join(REDIRECT)).map_err(|e| {
        let _ = std::fs::remove_dir_all(&to);
        format!("could not add the driver: {e}")
    })?;
    Ok(Driver { id, meta: manifest })
}

/// `META` at the archive's root (""), or one folder down ("folder/").
pub fn manifest_prefix(names: &[String]) -> Option<String> {
    if names.iter().any(|n| n == META) {
        return Some(String::new());
    }
    names.iter().find_map(|n| {
        let (top, rest) = n.split_once('/')?;
        (rest == META).then(|| format!("{top}/"))
    })
}

/// The package's manifest, when it names a driver.
pub fn read_manifest(path: &Path, prefix: &str) -> Option<Metadata> {
    let mut archive = zip::ZipArchive::new(File::open(path).ok()?).ok()?;
    let mut entry = archive.by_name(&format!("{prefix}{META}")).ok()?;
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    let meta: Metadata = serde_json::from_str(&text).ok()?;
    if meta.name.trim().is_empty() || !validate_library_name(&meta.library_name) {
        return None;
    }
    Some(meta)
}

/// A folder name made of `title` that `dest` has no folder of yet.
fn unique_title_in(title: &str, dest: &Path) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_owned();
    let cleaned = if cleaned.is_empty() {
        "Driver".to_owned()
    } else {
        cleaned
    };
    if !dest.join(&cleaned).exists() {
        return cleaned;
    }
    (2..)
        .map(|n| format!("{cleaned} ({n})"))
        .find(|name| !dest.join(name).exists())
        .expect("an unused name")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn elf_head(machine: u16, class: u8) -> Vec<u8> {
        let mut head = vec![0u8; 64];
        head[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        head[4] = class;
        head[18..20].copy_from_slice(&machine.to_le_bytes());
        head
    }

    #[test]
    fn only_plain_so_names_load() {
        assert!(validate_library_name("vulkan.ad07XX.so"));
        for bad in [
            "",
            "vulkan.so/",
            "../vulkan.so",
            "sub/vulkan.so",
            "vulkan\\x.so",
            "vulkan.dll",
            "vulkan.so ",
        ] {
            assert!(!validate_library_name(bad), "{bad}");
        }
    }

    #[test]
    fn only_64_bit_arm_libraries_load() {
        assert!(is_elf_aarch64(&elf_head(183, 2)));
        assert!(!is_elf_aarch64(&elf_head(62, 2))); // x86-64
        assert!(!is_elf_aarch64(&elf_head(40, 2))); // 32-bit ARM
        assert!(!is_elf_aarch64(&elf_head(183, 1))); // 32-bit ELF
        assert!(!is_elf_aarch64(&[0x7f, b'E', b'L']));
    }

    #[test]
    fn manifests_are_found_at_the_root_or_one_down() {
        assert_eq!(
            manifest_prefix(&["meta.json".into(), "vulkan.so".into()]),
            Some(String::new())
        );
        assert_eq!(
            manifest_prefix(&["Top/meta.json".into()]),
            Some("Top/".to_owned())
        );
        assert_eq!(manifest_prefix(&["readme.txt".into()]), None);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("horizon-gpu-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn driver_zip(path: &Path, prefix: &str, library: &[u8]) {
        let mut z = zip::ZipWriter::new(File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        let manifest = r#"{"name":"Turnip","driverVersion":"v25","minApi":28,"libraryName":"vulkan.ad07XX.so"}"#;
        z.start_file(format!("{prefix}{META}"), options).unwrap();
        z.write_all(manifest.as_bytes()).unwrap();
        z.start_file(format!("{prefix}vulkan.ad07XX.so"), options)
            .unwrap();
        z.write_all(library).unwrap();
        // Hostile and nested paths land flat, by file name, or not at all.
        z.start_file(format!("{prefix}sub/real.so"), options)
            .unwrap();
        z.write_all(b"nested").unwrap();
        z.finish().unwrap();
    }

    #[test]
    fn packages_install_flat_by_title() {
        let dir = scratch("install");
        let package = dir.join("turnip.adpkg");
        driver_zip(&package, "", &elf_head(183, 2));
        let dest = dir.join("gpu-drivers");
        let first = install_into(&package, "Turnip", &dest).unwrap();
        assert_eq!(first.id, "Turnip");
        assert!(dest.join("Turnip/vulkan.ad07XX.so").is_file());
        assert!(dest.join("Turnip/real.so").is_file());
        assert!(dest.join("Turnip").join(REDIRECT).is_dir());
        // A hostile `..` entry never escapes the folder.
        assert!(!dest.join("evil.so").exists());
        let second = install_into(&package, "Turnip", &dest).unwrap();
        assert_eq!(second.id, "Turnip (2)");
        let missing = dir.join("empty.adpkg");
        driver_zip(&missing, "", b"short");
        assert!(install_into(&missing, "Short", &dest).is_err());
    }

    #[test]
    fn repeated_crashes_set_the_driver_aside() {
        let dir = scratch("starts");
        let package = dir.join("turnip.adpkg");
        driver_zip(&package, "", &elf_head(183, 2));
        install_into(&package, "Turnip", &dir).unwrap();
        select_in("Turnip", &dir);
        assert_eq!(selected_in(&dir).as_deref(), Some("Turnip"));
        assert!(note_start_in(&dir));
        assert!(note_start_in(&dir));
        assert!(!note_start_in(&dir));
        assert_eq!(selected_in(&dir), None);
        let notice = take_notice_in(&dir).expect("a notice");
        assert!(notice.contains("Turnip"), "{notice}");
        assert_eq!(take_notice_in(&dir), None);
        // The system driver always starts.
        assert!(note_start_in(&dir));
    }

    #[test]
    fn a_first_frame_clears_the_crash_count() {
        let dir = scratch("first-frame");
        let package = dir.join("turnip.adpkg");
        driver_zip(&package, "", &elf_head(183, 2));
        install_into(&package, "Turnip", &dir).unwrap();
        select_in("Turnip", &dir);
        assert!(note_start_in(&dir));
        note_first_frame_in(&dir);
        assert!(note_start_in(&dir));
        assert!(note_start_in(&dir));
        assert_eq!(selected_in(&dir).as_deref(), Some("Turnip"));
    }

    #[test]
    fn shadowed_libraries_are_rejected() {
        let dir = scratch("reject");
        // Same file name in another folder would overwrite the validated library: rejected.
        let evil = dir.join("evil.adpkg");
        let mut z = zip::ZipWriter::new(File::create(&evil).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        let manifest = r#"{"name":"Evil","libraryName":"vulkan.evil.so"}"#;
        z.start_file(META, options).unwrap();
        z.write_all(manifest.as_bytes()).unwrap();
        z.start_file("vulkan.evil.so", options).unwrap();
        z.write_all(&elf_head(183, 2)).unwrap();
        z.start_file("nested/vulkan.evil.so", options).unwrap();
        z.write_all(&[0u8; 64]).unwrap();
        z.finish().unwrap();
        let error = install_into(&evil, "Evil", &dir).expect_err("shadowed library");
        assert!(error.contains("twice"), "{error}");
        assert!(!dir.join("Evil").exists());
    }
}
