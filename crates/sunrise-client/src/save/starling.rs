//! Starling's part of save.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Starling: a file's new bytes (`fs::write` through `_new` and a rename); queued like `write_dat`.
pub fn write_file(path: &Path, bytes: Vec<u8>) -> Result<()> {
    if later::is_deferred() {
        later::queue(path, later::Job::Bytes(bytes));
        return Ok(());
    }
    later::flush();
    write_file_now(path, &bytes)
}

pub(super) fn write_file_now(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let temporary = path.with_file_name(format!("{file}_new"));
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
}

/// Starling: a file's bytes, the newest queued ones first.
pub fn read_file(path: &Path) -> std::io::Result<Vec<u8>> {
    match later::queued(path) {
        Some(bytes) => Ok(bytes),
        None => fs::read(path),
    }
}
