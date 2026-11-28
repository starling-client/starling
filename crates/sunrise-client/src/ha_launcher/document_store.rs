//! The game folder's allowlisted folders as document ids, for the system's file manager, and the
//! launcher's own file manager over a profile's folder. Independent of Android and the game.
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

pub const ROOT: &str = "sunrise";
/// The most entries the file manager lists in one folder.
const MAX_CHILDREN: usize = 32768;
const CONFIG_FILES: &[&str] = &[
    "options.txt",
    "sunrise-graphics.txt",
    "sunrise-touch.json",
    "sunrise-keypad.json",
];
fn invalid() -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, "Invalid Sunrise document")
}
fn component(name: &str) -> io::Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.starts_with('.')
        || name.len() > 240
        || name.chars().any(|c| c.is_control() || "/\\:".contains(c))
    {
        return Err(invalid());
    }
    Ok(())
}

pub fn folder_id(profile: u32, folder: &str) -> String {
    if folder == "reports" {
        "reports".into()
    } else {
        format!("p{profile}/{folder}")
    }
}
pub fn profile(home: &Path) -> u32 {
    fs::read(home.join("android-launcher.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["profile"].as_u64())
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0)
}
/// The folder or file a document ID names in the game folder `home`, for the system's file
/// manager: [`ROOT`] for the folder itself, `p<profile>/<folder>[/<name>...]` below it. The
/// reports (`folder_id`'s "reports") are the app's own, which `platform::open_documents` opens.
pub fn path_of(home: &Path, id: &str) -> Option<PathBuf> {
    if id == ROOT {
        return Some(home.to_owned());
    }
    resolve(home, id).ok()
}
fn resolve(home: &Path, id: &str) -> io::Result<PathBuf> {
    let mut parts = id.split('/');
    let first = parts.next().ok_or_else(invalid)?;
    let (base, label) = {
        let number = first.strip_prefix('p').ok_or_else(invalid)?;
        let profile: u32 = number.parse().map_err(|_| invalid())?;
        // IDs are canonical and remain tied to their profile after switching.
        if format!("p{profile}") != first {
            return Err(invalid());
        }
        let root = if profile == 0 {
            home.to_owned()
        } else {
            home.join("profiles").join(format!("profile-{profile}"))
        };
        let (folder, label) = match parts.next() {
            Some("mods") => ("sunrise-mods", "Mods"),
            Some("packs") => ("resourcepacks", "Resource Packs"),
            Some("shaders") => ("shaderpacks", "Shader Packs"),
            Some("worlds") => ("saves", "Worlds"),
            Some("configuration") => ("", "Configuration"),
            _ => return Err(invalid()),
        };
        (root.join(folder), label)
    };
    let tail: Vec<_> = parts.collect();
    let configuration = label == "Configuration";
    if configuration && !tail.is_empty() && (tail.len() != 1 || !CONFIG_FILES.contains(&tail[0])) {
        return Err(invalid());
    }
    for name in &tail {
        component(name)?;
    }
    let mut path = base.clone();
    for name in &tail {
        path.push(name);
    }
    // Reject every symlink, including a replaced profile or allowlisted root.
    let relative = path.strip_prefix(home).map_err(|_| invalid())?;
    let mut checked = home.to_owned();
    for part in relative.components() {
        checked.push(part);
        match fs::symlink_metadata(&checked) {
            Ok(m) if m.file_type().is_symlink() => return Err(invalid()),
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    if tail.is_empty() {
        fs::create_dir_all(&base)?;
    }
    Ok(path)
}
pub const PROTECTED_SYSTEM_NAMES: &[&str] = &[
    "android-launcher.json",
    "android-launcher.partial",
    "engine-data.zip",
    ".local",
    "account.json",
    "accounts.json",
    "sessions",
    "installer",
    "wardrobe",
    "profiles",
];

pub const FM_ROOT_DIRS: &[(&str, &str)] = &[
    ("", "Root"),
    ("saves", "Worlds"),
    ("sunrise-mods", "Mods"),
    ("resourcepacks", "Packs"),
    ("shaderpacks", "Shaders"),
    ("controls", "Controls"),
    ("screenshots", "Shots"),
    ("performance", "Reports"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FmEntry {
    pub name: String,
    pub rel_path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub modified_secs: u64,
    pub kind_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FmFileDetails {
    pub name: String,
    pub rel_path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub modified_label: String,
    pub kind_label: String,
    pub preview_lines: Vec<String>,
}

pub fn is_protected_system_name(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    lower.starts_with('.')
        || PROTECTED_SYSTEM_NAMES
            .iter()
            .any(|p| p.eq_ignore_ascii_case(&lower))
}

pub fn is_protected_root_folder(rel_path: &str) -> bool {
    let clean = rel_path.trim_matches('/');
    clean.is_empty()
        || matches!(
            clean,
            "saves"
                | "sunrise-mods"
                | "resourcepacks"
                | "shaderpacks"
                | "controls"
                | "screenshots"
                | "performance"
        )
}

pub fn validate_entry_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Name cannot be empty.".into());
    }
    if trimmed == "." || trimmed == ".." || trimmed.contains("..") {
        return Err("Path traversal ('..') is not allowed.".into());
    }
    if trimmed.starts_with('.') {
        return Err("Hidden or system names starting with '.' are not allowed.".into());
    }
    if trimmed.len() > 200 {
        return Err("Name is too long.".into());
    }
    if trimmed.chars().any(|c| {
        c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    }) {
        return Err("Name cannot contain slashes or special path characters.".into());
    }
    if is_protected_system_name(trimmed) {
        return Err("This name is reserved by the Sunrise launcher.".into());
    }
    Ok(())
}

fn verify_no_symlinks(home: &Path, target: &Path) -> Result<(), String> {
    let rel = target
        .strip_prefix(home)
        .map_err(|_| "Path escapes the Sunrise home directory.".to_owned())?;
    let mut cur = home.to_path_buf();
    for part in rel.components() {
        use std::path::Component;
        match part {
            Component::Normal(seg) => {
                let seg_str = seg.to_string_lossy();
                if seg_str == ".local" {
                    return Err("Access to internal engine storage (.local) is restricted.".into());
                }
                cur.push(seg);
            }
            Component::CurDir => continue,
            _ => return Err("Path traversal outside profile directory is forbidden.".into()),
        }
        match fs::symlink_metadata(&cur) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("Symbolic links are not permitted in Sunrise directories.".into());
            }
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

pub fn resolve_fm_path(home: &Path, profile_root: &Path, rel: &str) -> Result<PathBuf, String> {
    verify_no_symlinks(home, profile_root)?;
    let clean = rel.trim().trim_matches('/');
    if clean.is_empty() {
        return Ok(profile_root.to_path_buf());
    }
    let mut parts = clean.split('/');
    let first = parts.next().unwrap_or("");
    validate_entry_name(first)?;
    let mut path = if first == "performance" {
        home.join("performance")
    } else {
        profile_root.join(first)
    };
    for seg in parts {
        validate_entry_name(seg)?;
        path.push(seg);
    }
    verify_no_symlinks(home, &path)?;
    Ok(path)
}

pub fn ensure_fm_dirs(home: &Path, profile_root: &Path) {
    let _ = fs::create_dir_all(profile_root);
    for (sub, _) in FM_ROOT_DIRS {
        if sub.is_empty() {
            continue;
        }
        let target = if *sub == "performance" {
            home.join("performance")
        } else {
            profile_root.join(sub)
        };
        let _ = fs::create_dir_all(target);
    }
}

pub fn classify_entry_kind(path: &Path, is_dir: bool) -> String {
    if is_dir {
        return "Folder".into();
    }
    match path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("smod") => "Sunrise Mod (.smod)".into(),
        Some("hnmod") => "Native Mod (.hnmod)".into(),
        Some("jar") => "Java Mod (.jar)".into(),
        Some("sshader") => "Sunrise Shader (.sshader)".into(),
        Some("zip") => "Archive / Pack (.zip)".into(),
        Some("png") => "PNG Image (.png)".into(),
        Some("json") => "JSON Config (.json)".into(),
        Some("jsonl") => "Telemetry Log (.jsonl)".into(),
        Some("toml") => "TOML Config (.toml)".into(),
        Some("txt" | "cfg" | "properties") => "Text Config (.txt)".into(),
        Some("dat" | "mca" | "mcr") => "World Data".into(),
        Some(ext) => format!("{} File (.{ext})", ext.to_ascii_uppercase()),
        None => "File".into(),
    }
}

pub fn format_timestamp_utc(secs: u64) -> String {
    if secs == 0 {
        return "Unknown".into();
    }
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02} {hh:02}:{mm:02} UTC")
}

pub fn list_fm_dir(
    home: &Path,
    profile_root: &Path,
    rel_dir: &str,
) -> Result<Vec<FmEntry>, String> {
    ensure_fm_dirs(home, profile_root);
    let clean_rel = rel_dir.trim().trim_matches('/');
    let dir_path = resolve_fm_path(home, profile_root, clean_rel)?;
    let meta = fs::symlink_metadata(&dir_path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err("Target path is not a valid directory.".into());
    }
    let mut entries = Vec::new();
    let mut seen_names = std::collections::BTreeSet::new();
    for (idx, item) in fs::read_dir(&dir_path)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        if idx >= MAX_CHILDREN {
            break;
        }
        let Ok(item) = item else { continue };
        let file_name = item.file_name();
        let Some(name_str) = file_name.to_str() else {
            continue;
        };
        if validate_entry_name(name_str).is_err() {
            continue;
        }
        let Ok(smeta) = fs::symlink_metadata(item.path()) else {
            continue;
        };
        if smeta.file_type().is_symlink() {
            continue;
        }
        let is_dir = smeta.is_dir();
        if !is_dir && !smeta.is_file() {
            continue;
        }
        let child_rel = if clean_rel.is_empty() {
            name_str.to_owned()
        } else {
            format!("{clean_rel}/{name_str}")
        };
        let modified_secs = smeta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let size_bytes = if is_dir {
            fs::read_dir(item.path())
                .map(|rd| rd.count() as u64)
                .unwrap_or(0)
        } else {
            smeta.len()
        };
        seen_names.insert(name_str.to_owned());
        entries.push(FmEntry {
            name: name_str.to_owned(),
            rel_path: child_rel,
            is_dir,
            size_bytes,
            modified_secs,
            kind_label: classify_entry_kind(&item.path(), is_dir),
        });
    }
    if clean_rel.is_empty() && !seen_names.contains("performance") {
        let perf_dir = home.join("performance");
        if let Ok(smeta) = fs::symlink_metadata(&perf_dir)
            && smeta.is_dir()
            && !smeta.file_type().is_symlink()
        {
            let count = fs::read_dir(&perf_dir)
                .map(|r| r.count() as u64)
                .unwrap_or(0);
            let modified_secs = smeta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            entries.push(FmEntry {
                name: "performance".into(),
                rel_path: "performance".into(),
                is_dir: true,
                size_bytes: count,
                modified_secs,
                kind_label: "Folder".into(),
            });
        }
    }
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        })
    });
    Ok(entries)
}

pub fn fm_mkdir(
    home: &Path,
    profile_root: &Path,
    parent_rel: &str,
    folder_name: &str,
) -> Result<String, String> {
    validate_entry_name(folder_name)?;
    let clean_parent = parent_rel.trim().trim_matches('/');
    let new_rel = if clean_parent.is_empty() {
        folder_name.trim().to_owned()
    } else {
        format!("{clean_parent}/{}", folder_name.trim())
    };
    let target = resolve_fm_path(home, profile_root, &new_rel)?;
    if target.exists() {
        return Err(format!(
            "'{}' already exists in this folder.",
            folder_name.trim()
        ));
    }
    fs::create_dir(&target).map_err(|e| e.to_string())?;
    Ok(new_rel)
}

pub fn fm_rename(
    home: &Path,
    profile_root: &Path,
    target_rel: &str,
    new_name: &str,
) -> Result<String, String> {
    let clean_target = target_rel.trim().trim_matches('/');
    if is_protected_root_folder(clean_target) {
        return Err("Built-in profile root folders cannot be renamed.".into());
    }
    validate_entry_name(new_name)?;
    let src_path = resolve_fm_path(home, profile_root, clean_target)?;
    if !src_path.exists() {
        return Err("The selected item no longer exists.".into());
    }
    let parent_rel = clean_target.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    let dst_rel = if parent_rel.is_empty() {
        new_name.trim().to_owned()
    } else {
        format!("{parent_rel}/{}", new_name.trim())
    };
    let dst_path = resolve_fm_path(home, profile_root, &dst_rel)?;
    if dst_path == src_path {
        return Ok(dst_rel);
    }
    if dst_path.exists() {
        return Err(format!(
            "An item named '{}' already exists.",
            new_name.trim()
        ));
    }
    fs::rename(&src_path, &dst_path).map_err(|e| e.to_string())?;
    Ok(dst_rel)
}

fn verify_tree_no_symlinks(dir: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Err("Refusing to operate on symbolic links.".into());
    }
    if meta.is_dir() {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let sm = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if sm.file_type().is_symlink() {
                return Err("Directory contains a symbolic link.".into());
            }
            if sm.is_dir() {
                verify_tree_no_symlinks(&path)?;
            }
        }
    }
    Ok(())
}

pub fn fm_delete(
    home: &Path,
    profile_root: &Path,
    target_rels: &[String],
) -> Result<usize, String> {
    if target_rels.is_empty() {
        return Err("No items selected to delete.".into());
    }
    let mut deleted = 0usize;
    for rel in target_rels {
        let clean = rel.trim().trim_matches('/');
        if is_protected_root_folder(clean) {
            return Err(format!("Cannot delete protected folder '{clean}'."));
        }
        let path = resolve_fm_path(home, profile_root, clean)?;
        if let Some(fname) = path.file_name().and_then(|s| s.to_str())
            && is_protected_system_name(fname)
        {
            return Err(format!("Cannot delete protected launcher file '{fname}'."));
        }
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        };
        if meta.file_type().is_symlink() {
            return Err("Refusing to delete a symbolic link.".into());
        }
        if meta.is_dir() {
            verify_tree_no_symlinks(&path)?;
            fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
        } else {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        deleted += 1;
    }
    Ok(deleted)
}

pub fn unique_destination_path(dest_dir: &Path, original_name: &str) -> Result<PathBuf, String> {
    let direct = dest_dir.join(original_name);
    if !direct.exists() {
        return Ok(direct);
    }
    let p = Path::new(original_name);
    let stem = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(original_name);
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
    for i in 1..=10_000 {
        let candidate_name = if ext.is_empty() {
            format!("{stem} ({i})")
        } else {
            format!("{stem} ({i}).{ext}")
        };
        let candidate = dest_dir.join(candidate_name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("Too many duplicate filenames in destination folder.".into())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    verify_tree_no_symlinks(src)?;
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let child_src = entry.path();
        let child_name = entry.file_name();
        let child_dst = dst.join(child_name);
        let meta = fs::symlink_metadata(&child_src).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            return Err("Refusing to copy symbolic link.".into());
        }
        if meta.is_dir() {
            copy_dir_recursive(&child_src, &child_dst)?;
        } else if meta.is_file() {
            fs::copy(&child_src, &child_dst).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn fm_paste(
    home: &Path,
    profile_root: &Path,
    sources: &[PathBuf],
    is_cut: bool,
    dest_rel_dir: &str,
) -> Result<usize, String> {
    if sources.is_empty() {
        return Err("Clipboard is empty.".into());
    }
    let dest_dir = resolve_fm_path(home, profile_root, dest_rel_dir)?;
    let dest_meta = fs::symlink_metadata(&dest_dir).map_err(|e| e.to_string())?;
    if !dest_meta.is_dir() || dest_meta.file_type().is_symlink() {
        return Err("Destination is not a valid folder.".into());
    }
    let mut count = 0usize;
    for src in sources {
        verify_no_symlinks(home, src)?;
        let src_meta = fs::symlink_metadata(src).map_err(|e| e.to_string())?;
        if src_meta.file_type().is_symlink() {
            return Err("Refusing to copy or move symbolic links.".into());
        }
        let Some(fname) = src.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        validate_entry_name(fname)?;
        if src_meta.is_dir() && (dest_dir == *src || dest_dir.starts_with(src)) {
            return Err(format!(
                "Cannot {} folder '{fname}' into itself or its own subfolder.",
                if is_cut { "move" } else { "copy" }
            ));
        }
        if is_cut && src.parent() == Some(dest_dir.as_path()) {
            count += 1;
            continue;
        }
        let target = unique_destination_path(&dest_dir, fname)?;
        if is_cut {
            if fs::rename(src, &target).is_err() {
                if src_meta.is_dir() {
                    copy_dir_recursive(src, &target)?;
                    fs::remove_dir_all(src).map_err(|e| e.to_string())?;
                } else {
                    fs::copy(src, &target).map_err(|e| e.to_string())?;
                    fs::remove_file(src).map_err(|e| e.to_string())?;
                }
            }
        } else if src_meta.is_dir() {
            copy_dir_recursive(src, &target)?;
        } else {
            fs::copy(src, &target).map_err(|e| e.to_string())?;
        }
        count += 1;
    }
    Ok(count)
}

pub fn fm_details(
    home: &Path,
    profile_root: &Path,
    target_rel: &str,
) -> Result<FmFileDetails, String> {
    let clean = target_rel.trim().trim_matches('/');
    let path = resolve_fm_path(home, profile_root, clean)?;
    let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Err("Refusing to inspect symbolic link.".into());
    }
    let is_dir = meta.is_dir();
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(if clean.is_empty() {
            "Profile Root"
        } else {
            clean
        })
        .to_owned();
    let modified_secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let modified_label = format_timestamp_utc(modified_secs);
    let kind_label = classify_entry_kind(&path, is_dir);
    let mut size_bytes = meta.len();
    let mut preview_lines = Vec::new();
    if is_dir {
        let mut files_count = 0u64;
        let mut dirs_count = 0u64;
        let mut total_bytes = 0u64;
        if let Ok(rd) = fs::read_dir(&path) {
            for entry in rd.flatten() {
                if let Ok(m) = entry.metadata() {
                    if m.is_dir() {
                        dirs_count += 1;
                    } else if m.is_file() {
                        files_count += 1;
                        total_bytes = total_bytes.saturating_add(m.len());
                    }
                }
            }
        }
        size_bytes = total_bytes;
        preview_lines.push(format!(
            "Contains {dirs_count} folder(s) and {files_count} file(s)."
        ));
    } else {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        if matches!(
            ext.as_deref(),
            Some(
                "txt"
                    | "toml"
                    | "json"
                    | "jsonl"
                    | "cfg"
                    | "properties"
                    | "wgsl"
                    | "glsl"
                    | "fsh"
                    | "vsh"
                    | "md"
                    | "log"
            )
        ) {
            if let Ok(file) = File::open(&path) {
                use std::io::Read;
                let mut buf = Vec::new();
                if file.take(4096).read_to_end(&mut buf).is_ok() {
                    let text = String::from_utf8_lossy(&buf);
                    for line in text.lines().take(16) {
                        preview_lines.push(line.chars().take(120).collect());
                    }
                    if preview_lines.is_empty() {
                        preview_lines.push("(Empty text file)".into());
                    }
                }
            }
        } else {
            preview_lines.push(
                "Binary file (text preview is enabled for .txt, .toml, and .json files).".into(),
            );
        }
    }
    Ok(FmFileDetails {
        name,
        rel_path: if clean.is_empty() {
            "/".into()
        } else {
            clean.to_owned()
        },
        is_dir,
        size_bytes,
        modified_label,
        kind_label,
        preview_lines,
    })
}

#[cfg(test)]
mod fm_tests {
    use super::*;

    #[test]
    fn file_manager_operations_and_security_guards() {
        let home = std::env::temp_dir().join(format!(
            "sunrise-fm-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&home).unwrap();
        ensure_fm_dirs(&home, &home);

        // Protected system files must be rejected
        assert!(validate_entry_name("..").is_err());
        assert!(validate_entry_name("a/b").is_err());
        assert!(validate_entry_name("a\\b").is_err());
        assert!(validate_entry_name(".local").is_err());
        assert!(validate_entry_name("android-launcher.json").is_err());
        assert!(validate_entry_name("engine-data.zip").is_err());

        // Create folder & rename
        let created = fm_mkdir(&home, &home, "saves", "TestWorld").unwrap();
        assert_eq!(created, "saves/TestWorld");
        let renamed = fm_rename(&home, &home, "saves/TestWorld", "RenamedWorld").unwrap();
        assert_eq!(renamed, "saves/RenamedWorld");

        // Write a config/text file and verify details preview
        fs::write(
            home.join("saves/RenamedWorld/notes.txt"),
            "line1
line2
",
        )
        .unwrap();
        let details = fm_details(&home, &home, "saves/RenamedWorld/notes.txt").unwrap();
        assert_eq!(details.size_bytes, 12);
        assert!(details.preview_lines.iter().any(|l| l == "line1"));

        // Copy & Paste with duplicate filename handling (`notes (1).txt`)
        let src_file = home.join("saves/RenamedWorld/notes.txt");
        fm_paste(&home, &home, &[src_file], false, "saves/RenamedWorld").unwrap();
        assert!(home.join("saves/RenamedWorld/notes (1).txt").exists());

        // Prevent copying/moving a folder into itself or its own descendant
        let world_dir = home.join("saves/RenamedWorld");
        let sub_rel = fm_mkdir(&home, &home, "saves/RenamedWorld", "region").unwrap();
        assert!(fm_paste(&home, &home, &[world_dir], false, &sub_rel).is_err());

        // Delete file and folder
        let del_count = fm_delete(
            &home,
            &home,
            &["saves/RenamedWorld/notes (1).txt".to_owned()],
        )
        .unwrap();
        assert_eq!(del_count, 1);
        assert!(!home.join("saves/RenamedWorld/notes (1).txt").exists());

        // Root folders cannot be deleted
        assert!(fm_delete(&home, &home, &["saves".to_owned()]).is_err());

        fs::remove_dir_all(home).unwrap();
    }
}
