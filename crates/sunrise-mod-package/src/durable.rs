//! Durable file publication for saves and other persistent game state.
//!
//! A file's contents are made durable by writing a temporary sibling,
//! `sync_all`ing it, and moving it over the destination with
//! `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)` on
//! Windows — which returns only once the rename metadata is flushed — or
//! `rename` plus an `fsync` of the directory on Unix. Every API failure is
//! an error; nothing silently falls back to a weaker write.
//!
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{fs, fs::File};

#[cfg(any(unix, windows))]
fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// Syncs the directory containing `path`, including any rename into it (Unix).
/// On Windows [`replace`] instead uses a write-through move.
pub fn sync_parent(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(parent(path))?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Moves a synced `from` onto `to` and makes the destination durable.
/// Both must live on the same filesystem; `to` may not exist yet.
pub fn replace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        // The launcher's Windows suite compiles this file under edition 2021, which sorts
        // these names differently from the 2024 crates: keep one order for both.
        #[rustfmt::skip]
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        // std adds the extended-length prefix for long paths; retain that
        // behavior when calling Win32 directly, including a destination
        // that does not exist yet.
        let from = fs::canonicalize(from)?;
        let name = to.file_name().ok_or(io::ErrorKind::InvalidInput)?;
        let to = fs::canonicalize(parent(to))?.join(name);
        let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        // Both paths are owned, nul-terminated buffers valid for this
        // synchronous call.
        if unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    #[cfg(not(windows))]
    fs::rename(from, to)?;
    sync_parent(to)
}

/// Writes `bytes` to `path` so that, once this returns, the file on disk
/// holds exactly these bytes: a temporary sibling, `sync_all`, then a
/// durable [`replace`]. A failed publication removes the temporary.
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_with(path, |file| file.write_all(bytes))
}

/// Publishes `path` only after `write` has finished writing its temporary sibling.
pub fn write_with(path: &Path, write: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    for _ in 0..16 {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let temporary =
            path.with_file_name(format!(".sunrise-write-{}-{id}.tmp", std::process::id()));
        let mut file = match File::create_new(&temporary) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let written = write(&mut file).and_then(|()| file.sync_all());
        drop(file);
        return written
            .and_then(|()| replace(&temporary, path))
            .inspect_err(|_| {
                let _ = fs::remove_file(&temporary);
            });
    }
    Err(io::ErrorKind::AlreadyExists.into())
}

/// Removes `path`, syncing its parent directory on Unix. A missing file is already gone.
/// Windows does not flush the unlink here: a journal removed this way can reappear after
/// power loss, so the caller must be able to replay it safely.
pub fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => sync_parent(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_whole_and_leaves_no_temporary_file() {
        let name = "durable-whole";
        let dir =
            std::env::temp_dir().join(format!("sunrise-private-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("api.json");
        write(&path, b"one").unwrap();
        write(&path, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        remove(&path).unwrap();
        assert!(!path.exists());
        remove(&path).unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
