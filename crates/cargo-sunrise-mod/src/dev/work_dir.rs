use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The project's work folder in the cache, by mod id and the project's own (canonical) folder: two
/// checkouts of one mod (Git worktrees, a fork beside the original) never share a staged copy, its
/// lock or the cargo build folder kept by its path, so one's check neither waits for the other's
/// nor recompiles the mod after it.
pub(crate) fn work_dir(cache: &Path, id: &str, project: &Path) -> PathBuf {
    let hash = Sha256::digest(project.as_os_str().as_encoded_bytes());
    let short: String = hash[..6].iter().map(|b| format!("{b:02x}")).collect();
    cache.join("work").join(format!("{id}-{short}"))
}
