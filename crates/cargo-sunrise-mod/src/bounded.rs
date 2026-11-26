//! File limits at the developer-tool input boundary.
use std::{
    fmt, fs,
    io::{self, Read},
    path::{Path, PathBuf},
};
pub use sunrise_mod_package::manifest::MAX_MANIFEST;
// Bound the decoded internals catalog before parsing it.
pub const MAX_CATALOG: u64 = 256 << 20;
// The largest checked-in slot list is sunrise.txt (2,839 bytes). Lists contain
// declarations, not source; 4 MiB leaves substantial room for mod declarations.

#[derive(Debug)]
pub enum ReadError {
    Io(PathBuf, io::Error),
    NotRegular(PathBuf),
    TooLarge(PathBuf, u64),
}
impl ReadError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Io(_, e) if e.kind() == io::ErrorKind::NotFound)
    }
}
impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(p, e) => write!(f, "{}: {e}", p.display()),
            Self::NotRegular(p) => write!(f, "{}: not a regular file", p.display()),
            Self::TooLarge(p, limit) => write!(f, "{}: exceeds {limit} byte limit", p.display()),
        }
    }
}
impl std::error::Error for ReadError {}
impl From<ReadError> for String {
    fn from(e: ReadError) -> Self {
        e.to_string()
    }
}
pub fn read(path: &Path, limit: u64) -> Result<String, ReadError> {
    String::from_utf8(read_bytes(path, limit, Symlinks::Follow)?).map_err(|e| {
        ReadError::Io(
            path.to_owned(),
            io::Error::new(io::ErrorKind::InvalidData, e),
        )
    })
}

/// Whether the final component may be a symlink to a regular file.
pub enum Symlinks {
    Follow,
    Refuse,
}

pub fn read_bytes(path: &Path, limit: u64, symlinks: Symlinks) -> Result<Vec<u8>, ReadError> {
    let io_error = |e| ReadError::Io(path.to_owned(), e);
    let metadata = match symlinks {
        Symlinks::Follow => fs::metadata(path),
        Symlinks::Refuse => fs::symlink_metadata(path),
    }
    .map_err(io_error)?;
    if !metadata.is_file() {
        return Err(ReadError::NotRegular(path.to_owned()));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    // Nonblocking open also protects against a regular file being swapped for a FIFO.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let no_follow = match symlinks {
            Symlinks::Follow => 0,
            Symlinks::Refuse => libc::O_NOFOLLOW,
        };
        options.custom_flags(libc::O_NONBLOCK | no_follow);
    }
    let file = options.open(path).map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(ReadError::NotRegular(path.to_owned()));
    }
    read_limited(file, limit, path)
}
/// Reads at most `limit + 1` bytes, so an endless or huge input is refused without being buffered.
pub(crate) fn read_limited(
    reader: impl Read,
    limit: u64,
    path: &Path,
) -> Result<Vec<u8>, ReadError> {
    let io_error = |e| ReadError::Io(path.to_owned(), e);
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > limit {
        return Err(ReadError::TooLarge(path.to_owned(), limit));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_exact_limit_and_typed_error() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/bounded-exact.txt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "abcd").unwrap();
        assert_eq!(read(&path, 4).unwrap(), "abcd");
        assert!(matches!(read(&path, 3), Err(ReadError::TooLarge(ref p, 3)) if p == &path));
        fs::remove_file(path).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn text_reads_accept_user_symlinks_but_binary_reads_can_refuse_them() {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("manifest");
        let path = work.path().join("symlink");
        fs::write(&source, "text").unwrap();
        std::os::unix::fs::symlink(source, &path).unwrap();
        assert_eq!(read(&path, 4).unwrap(), "text");
        assert!(
            matches!(read_bytes(&path, 4, Symlinks::Refuse), Err(ReadError::NotRegular(p)) if p == path)
        );
    }

    /// Hands out `b'a'` forever, but fails any read after `cap` bytes were taken.
    struct Endless {
        taken: u64,
        cap: u64,
    }
    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.taken >= self.cap {
                return Err(io::Error::other("read past the bound"));
            }
            let n = buf.len().min((self.cap - self.taken) as usize);
            buf[..n].fill(b'a');
            self.taken += n as u64;
            Ok(n)
        }
    }
    #[test]
    fn an_endless_reader_is_refused_after_limit_plus_one_bytes() {
        let path = Path::new("endless");
        // `cap` is limit + 1: with `take` the reader is never polled again; without it the next read fails.
        let reader = Endless { taken: 0, cap: 4 };
        assert!(matches!(
            read_limited(reader, 3, path),
            Err(ReadError::TooLarge(_, 3))
        ));
        let at_the_limit = io::Cursor::new(b"aaa".to_vec());
        assert_eq!(read_limited(at_the_limit, 3, path).unwrap(), b"aaa");
    }
    #[test]
    fn the_sentinel_byte_does_not_overflow_at_the_largest_limit() {
        let reader = io::Cursor::new(b"ab".to_vec());
        assert_eq!(
            read_limited(reader, u64::MAX, Path::new("max")).unwrap(),
            b"ab"
        );
    }
}
