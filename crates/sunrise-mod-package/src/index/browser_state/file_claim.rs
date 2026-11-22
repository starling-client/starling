use std::ffi::OsStr;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Component, Path};

/// A claim anchored to the real directory that contains its file or missing suffix.
pub(super) struct FileClaim {
    parent: Identity,
    // Preserve the known prefix even when a later component cannot be compared.
    tail: Vec<Option<String>>,
    spelling: Option<Vec<String>>,
    file: Option<Identity>,
}

pub(super) enum Conflict {
    Confirmed,
    Possible,
}

impl FileClaim {
    pub(super) fn target(home: &Path, claim: &str) -> Result<Self, String> {
        let mut depth = 0;
        let mut parts = claim
            .split(|c| c == '/' || (cfg!(windows) && c == '\\'))
            .peekable();
        while let Some(part) = parts.next() {
            match part {
                "." if parts.peek().is_some() => {}
                ".." if depth > 0 && parts.peek().is_some() => depth -= 1,
                _ if portable_name(part) => depth += 1,
                _ => return Err(format!("invalid destination component {part:?}")),
            }
        }
        Self::read(home, claim, true)
    }

    pub(super) fn other(home: &Path, claim: &str) -> Result<Self, String> {
        Self::read(home, claim, false)
    }

    fn read(home: &Path, claim: &str, target: bool) -> Result<Self, String> {
        let path = home.join(claim);
        // Another row may use an absolute path. Use the filesystem's root in that case,
        // not this home; a real alias can be reached from either starting point.
        let (base, relative, mut spelling) = match path.strip_prefix(home) {
            Ok(relative) => (home, relative, home_spelling(relative)),
            Err(_) if path.is_absolute() => {
                let root = path.ancestors().last().unwrap_or(&path);
                (
                    root,
                    path.strip_prefix(root).map_err(|e| e.to_string())?,
                    None,
                )
            }
            Err(_) => return Err("cannot locate the file claim's starting directory".into()),
        };
        let mut parent = base.to_path_buf();
        let mut parent_info = None;
        let mut parts = relative.components().peekable();
        let tracks_parents = parts.clone().any(|part| part == Component::ParentDir);
        let mut resolved_links = Vec::new();
        let mut tail = Vec::new();
        let mut file = None;
        while let Some(part) = parts.next() {
            let leaf = parts.peek().is_none();
            if part == Component::ParentDir && !leaf && tail.pop().is_some() {
                continue;
            }
            let name = part.as_os_str();
            if !tail.is_empty() {
                tail.push(fold_unresolved(name));
                continue;
            }
            let current = parent.join(name);
            let metadata = if leaf && target {
                fs::symlink_metadata(&current)
            } else {
                fs::metadata(&current)
            };
            match metadata {
                Ok(info) if leaf => {
                    if target && (!info.is_file() || is_link(&info)) {
                        return Err(format!("{} is not a plain file", current.display()));
                    }
                    if info.is_file() {
                        file = Some(
                            identity(&current, Some(&info))
                                .map_err(|e| format!("{}: {e}", current.display()))?,
                        );
                    }
                    tail.push(
                        name.to_str()
                            .filter(|name| portable_name(name))
                            .map(str::to_ascii_lowercase),
                    );
                }
                Ok(info) if info.is_dir() => {
                    if spelling.is_some() && tracks_parents {
                        match part {
                            Component::Normal(_) => {
                                let no_follow = fs::symlink_metadata(&current)
                                    .map_err(|e| format!("{}: {e}", current.display()))?;
                                resolved_links.push(is_link(&no_follow));
                            }
                            Component::ParentDir if resolved_links.pop() == Some(true) => {
                                // The lexical pop does not name the native parent of a link.
                                spelling = None;
                            }
                            _ => {}
                        }
                    }
                    parent = current;
                    parent_info = Some(info);
                }
                Ok(_) if !target => tail.push(None),
                Ok(_) => return Err(format!("{} is not a directory", current.display())),
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    // Do not mistake a dangling link for a new filename in this directory.
                    // We cannot associate a dangling link with a checked destination.
                    match fs::symlink_metadata(&current) {
                        Err(e) if e.kind() == ErrorKind::NotFound => {
                            tail.push(fold_unresolved(name))
                        }
                        Err(e) if !target && invalid_lookup(&e) => tail.push(None),
                        Err(e) => return Err(format!("{}: {e}", current.display())),
                        Ok(_) => return Err(format!("cannot resolve {}", current.display())),
                    }
                }
                // No file can be reached through a rejected name or a non-directory.
                // This does not make rows under other directories ambiguous.
                Err(e) if !target && invalid_lookup(&e) => tail.push(None),
                // For example, permission denied may hide a link; no identity to bound it.
                Err(e) => return Err(format!("{}: {e}", current.display())),
            }
        }
        if tail.is_empty() {
            // The other row names a directory root rather than a file.
            tail.push(None);
        }
        let parent = identity(&parent, parent_info.as_ref())
            .map_err(|e| format!("{}: {e}", parent.display()))?;
        Ok(Self {
            parent,
            tail,
            spelling,
            file,
        })
    }

    pub(super) fn conflict_with(&self, other: &Self) -> Option<Conflict> {
        if (self.file.is_some() && self.file == other.file)
            || self
                .spelling
                .as_ref()
                .zip(other.spelling.as_ref())
                .is_some_and(|(a, b)| a.starts_with(b) || b.starts_with(a))
        {
            return Some(Conflict::Confirmed);
        }
        if self.parent != other.parent {
            return None;
        }
        for (a, b) in self.tail.iter().zip(&other.tail) {
            match (a, b) {
                (Some(a), Some(b)) if a == b => {}
                (Some(_), Some(_)) => return None,
                _ => return Some(Conflict::Possible),
            }
        }
        // Equal suffixes, or one file claims another's directory.
        Some(Conflict::Confirmed)
    }
}

fn fold_unresolved(name: &OsStr) -> Option<String> {
    let name = name.to_str()?;
    (name.is_ascii() && !name.contains('~') && portable_name(name))
        .then(|| name.to_ascii_lowercase())
}

/// Check portable spellings even when one path resolves farther than the other. Discard
/// this lexical key if native lookup finds that `..` crosses a link.
fn home_spelling(relative: &Path) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for part in relative.components() {
        match part {
            Component::Normal(name) => names.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                names.pop()?;
            }
            _ => return None,
        }
    }
    if names.is_empty() {
        return None;
    }
    names.into_iter().map(fold_unresolved).collect()
}

#[cfg(unix)]
type Identity = (u64, u64);
#[cfg(not(unix))]
type Identity = same_file::Handle;

#[cfg(unix)]
fn identity(path: &Path, info: Option<&fs::Metadata>) -> io::Result<Identity> {
    use std::os::unix::fs::MetadataExt;
    // A stat gives the same identity without requiring permission to read the file.
    let read;
    let info = match info {
        Some(info) => info,
        None => {
            read = fs::metadata(path)?;
            &read
        }
    };
    Ok((info.dev(), info.ino()))
}

#[cfg(not(unix))]
fn identity(path: &Path, _info: Option<&fs::Metadata>) -> io::Result<Identity> {
    same_file::Handle::from_path(path)
}

fn is_link(info: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        info.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        info.is_symlink()
    }
}

fn invalid_lookup(error: &io::Error) -> bool {
    // ERROR_INVALID_NAME is not consistently mapped to InvalidInput in std.
    #[cfg(windows)]
    if error.raw_os_error() == Some(windows_sys::Win32::Foundation::ERROR_INVALID_NAME as i32) {
        return true;
    }
    matches!(
        error.kind(),
        ErrorKind::InvalidInput | ErrorKind::NotADirectory
    )
}

// Apply Win32's filename restrictions to destinations even on Unix, since homes can be moved.
fn portable_name(part: &str) -> bool {
    if part.is_empty()
        || part.ends_with(['.', ' '])
        || part
            .bytes()
            .any(|b| b < b' ' || b"<>:\"/\\|?*".contains(&b))
    {
        return false;
    }
    let (stem, _) = part.split_once('.').unwrap_or((part, ""));
    let stem = stem.trim_end_matches(' ').to_ascii_uppercase();
    !(matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        .is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        }))
}
