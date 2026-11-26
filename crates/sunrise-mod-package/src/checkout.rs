//! Bounded reads of regular files below a trusted checkout root.
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

/// Only descend through real directories under the trusted checkout root. The same policy
/// covers entry directories, caches, and images at any depth.
pub(crate) fn checkout_path(root: &Path, relative: &Path, directory: bool) -> io::Result<PathBuf> {
    let invalid_path = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "expected a relative checkout path",
        )
    };
    let mut path = root.to_owned();
    let mut components = relative.components().peekable();
    if components.peek().is_none() {
        return Err(invalid_path());
    }
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(invalid_path());
        };
        // Push only a single relative name, not a prefix reinterpreted by the host filesystem.
        if !matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        ) {
            return Err(invalid_path());
        }
        path.push(name);
        let meta = std::fs::symlink_metadata(&path)?;
        let want_dir = directory || components.peek().is_some();
        if (want_dir && !meta.is_dir()) || (!want_dir && !meta.is_file()) {
            let expected = if want_dir { "directory" } else { "file" };
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not a regular {expected}", path.display()),
            ));
        }
    }
    Ok(path)
}

pub(crate) fn read_regular(root: &Path, relative: &Path, max: u64) -> io::Result<Vec<u8>> {
    let path = checkout_path(root, relative, false)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("exceeds {max} byte limit"),
        ));
    }
    Ok(bytes)
}
