//! Bind every signing path from its filesystem root. Never reopen an ambient path to do I/O.
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, DirBuilder, File, Metadata, OpenOptions};
use same_file::Handle;
use std::{
    ffi::{OsStr, OsString},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};
use sunrise_mod_package::manifest::{MANIFEST, NOT_PACKAGED};

pub(super) struct Directory {
    path: PathBuf,
    dir: Dir,
    // cap-std holds Windows directories without FILE_SHARE_DELETE, including every ancestor
    // here because some of its Windows operations recover a path from the handle.
    #[cfg(windows)]
    _ancestors: Vec<Dir>,
}

impl Directory {
    pub(super) fn open(path: &Path, key_output: bool) -> Result<Self, String> {
        if path.components().any(|part| part == Component::ParentDir) {
            return Err(format!("{}: use a path without '..'", path.display()));
        }
        let path = std::path::absolute(path).map_err(|e| at(path, e))?;
        let root: PathBuf = path
            .components()
            .take_while(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
            .collect();
        if !root.is_absolute() {
            return Err(format!("{}: expected a filesystem path", path.display()));
        }
        let mut dir =
            Dir::open_ambient_dir(&root, cap_std::ambient_authority()).map_err(|e| at(&root, e))?;
        reject_reparse(&dir.dir_metadata().map_err(|e| at(&root, e))?).map_err(|e| at(&root, e))?;
        let mut walked = root;
        let mut parts = path
            .components()
            .filter_map(|part| match part {
                Component::Normal(name) => Some(name),
                _ => None,
            })
            .peekable();
        #[cfg(windows)]
        let mut ancestors = Vec::new();
        loop {
            if key_output {
                match dir.symlink_metadata(MANIFEST) {
                    Ok(_)
                        if !parts
                            .peek()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| NOT_PACKAGED.contains(&name)) =>
                    {
                        return Err(format!(
                            "{} is inside the mod's package inputs: use .sunrise/ or a folder outside the project",
                            path.display()
                        ));
                    }
                    Ok(_) => (),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                    Err(e) => return Err(at(&walked.join(MANIFEST), e)),
                }
            }
            let Some(name) = parts.next() else { break };
            walked.push(name);
            let next = match dir.open_dir_nofollow(name) {
                Err(e) if key_output && e.kind() == io::ErrorKind::NotFound => {
                    match dir.create_dir_with(name, &private_dir()) {
                        Ok(()) => (),
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
                        Err(e) => return Err(at(&walked, e)),
                    }
                    dir.open_dir_nofollow(name)
                }
                result => result,
            }
            .map_err(|e| at(&walked, e))?;
            reject_reparse(&next.dir_metadata().map_err(|e| at(&walked, e))?)
                .map_err(|e| at(&walked, e))?;
            #[cfg(windows)]
            ancestors.push(dir);
            dir = next;
        }
        Ok(Self {
            path,
            dir,
            #[cfg(windows)]
            _ancestors: ancestors,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn require_private(&self, default: bool) -> Result<(), String> {
        #[cfg(unix)]
        {
            use cap_fs_ext::OsMetadataExt;
            let metadata = self.dir.dir_metadata().map_err(|e| at(&self.path, e))?;
            // SAFETY: geteuid takes no arguments and has no preconditions.
            let owner = unsafe { libc::geteuid() };
            let forbidden = if default { 0o077 } else { 0o022 };
            if metadata.uid() != owner || metadata.mode() & forbidden != 0 {
                return Err(format!(
                    "{}: use a signing directory you own with mode 0700",
                    self.path.display()
                ));
            }
        }
        #[cfg(not(unix))]
        let _ = default;
        Ok(())
    }

    pub(super) fn require_absent(&self, name: &OsStr) -> Result<(), String> {
        match self.dir.symlink_metadata(name) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(format!(
                "{} already exists; not overwritten",
                self.path.join(name).display()
            )),
            Err(e) => Err(at(&self.path.join(name), e)),
        }
    }

    pub(super) fn open_regular(&self, name: &OsStr) -> Result<File, String> {
        open_file(&self.dir, name, &read_options()).map_err(|e| at(&self.path.join(name), e))
    }

    pub(super) fn read(&self, name: &OsStr, limit: u64) -> Result<Vec<u8>, String> {
        let path = self.path.join(name);
        let file = self.open_regular(name)?;
        let mut bytes = Vec::new();
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| at(&path, e))?;
        if bytes.len() as u64 > limit {
            return Err(format!("{}: over {limit} bytes", path.display()));
        }
        Ok(bytes)
    }

    pub(super) fn write_new(&self, name: &OsStr, bytes: &[u8]) -> Result<(), String> {
        let mut stage = Staging::new(&self.dir).map_err(|e| at(&self.path, e))?;
        stage
            .write(name, bytes)
            .and_then(|_| stage.publish(name, &self.dir))
            .map_err(|e| at(&self.path.join(name), e))
    }

    pub(super) fn write_pair(
        &self,
        first: (&OsStr, &[u8]),
        second: (&OsStr, &[u8]),
    ) -> Result<(), String> {
        let mut stage = Staging::new(&self.dir).map_err(|e| at(&self.path, e))?;
        let identity = stage
            .write(first.0, first.1)
            .and_then(|file| Handle::from_file(file.into_std()))
            .map_err(|e| at(&self.path.join(first.0), e))?;
        stage
            .write(second.0, second.1)
            .map_err(|e| at(&self.path.join(second.0), e))?;
        self.publish_pair(&stage, first.0, &identity, second.0)
    }

    fn publish_pair(
        &self,
        stage: &Staging,
        first: &OsStr,
        identity: &Handle,
        second: &OsStr,
    ) -> Result<(), String> {
        stage
            .publish(first, &self.dir)
            .map_err(|e| at(&self.path.join(first), e))?;
        if let Err(e) = stage.publish(second, &self.dir) {
            let error = at(&self.path.join(second), e);
            if let Err(rollback) = remove_ours(&self.dir, first, identity) {
                return Err(format!(
                    "{error}; could not roll back {}: {rollback}",
                    self.path.join(first).display()
                ));
            }
            return Err(error);
        }
        Ok(())
    }
}

fn at(path: &Path, error: io::Error) -> String {
    format!("{}: {error}", path.display())
}

fn reject_reparse(_metadata: &Metadata) -> io::Result<()> {
    #[cfg(windows)]
    {
        use cap_fs_ext::OsMetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if _metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reparse points are not allowed",
            ));
        }
    }
    Ok(())
}

fn read_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    options
}

fn open_file(dir: &Dir, name: &OsStr, options: &OpenOptions) -> io::Result<File> {
    let file = dir.open_with(name, options)?;
    let metadata = file.metadata()?;
    reject_reparse(&metadata)?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a regular file",
        ));
    }
    Ok(file)
}

fn remove_ours(dir: &Dir, name: &OsStr, original: &Handle) -> io::Result<()> {
    let options = read_options();
    #[cfg(windows)]
    let options = {
        use cap_std::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            DELETE, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        let mut options = options;
        options
            .access_mode(DELETE | FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
        options
    };
    let file = match open_file(dir, name, &options) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        result => result?,
    };
    let current = Handle::from_file(file.into_std())?;
    if current != *original {
        return Ok(());
    }
    #[cfg(unix)]
    {
        // The key directory is owned by the caller and not writable by other users.
        // There is no portable identity-conditional unlink on Unix.
        dir.remove_file(name)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
            FILE_DISPOSITION_INFO_EX, FileDispositionInfoEx, SetFileInformationByHandle,
        };
        let info = FILE_DISPOSITION_INFO_EX {
            Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
        };
        // SAFETY: the verified open file and initialized record remain live for the call.
        let ok = unsafe {
            SetFileInformationByHandle(
                current.as_file().as_raw_handle(),
                FileDispositionInfoEx,
                std::ptr::from_ref(&info).cast(),
                size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

fn private_dir() -> DirBuilder {
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    cap_std::fs::DirBuilderExt::mode(&mut builder, 0o700);
    #[cfg(not(unix))]
    let _ = &mut builder;
    builder
}

struct Staging {
    dir: Option<Dir>,
    files: Vec<OsString>,
}

impl Staging {
    fn new(destination: &Dir) -> io::Result<Self> {
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(io::Error::other)?;
        let name = format!(".sunrise-signing-{:032x}", u128::from_ne_bytes(random));
        destination.create_dir_with(&name, &private_dir())?;
        let dir = destination.open_dir_nofollow(&name)?;
        reject_reparse(&dir.dir_metadata()?)?;
        Ok(Self {
            dir: Some(dir),
            files: Vec::new(),
        })
    }

    fn dir(&self) -> &Dir {
        self.dir
            .as_ref()
            .expect("staging directory is open until drop")
    }

    fn write(&mut self, name: &OsStr, bytes: &[u8]) -> io::Result<File> {
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        #[cfg(unix)]
        cap_std::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = self.dir().open_with(name, &options)?;
        self.files.push(name.to_owned());
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(file)
    }

    fn publish(&self, name: &OsStr, destination: &Dir) -> io::Result<()> {
        // Like rename, only complete bytes become visible; unlike rename, a hard link never
        // replaces a name that another process created. Unsupported filesystems fail closed.
        self.dir().hard_link(name, destination, name)
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if let Some(dir) = self.dir.take() {
            for name in &self.files {
                let _ = dir.remove_file(name);
            }
            let _ = dir.remove_open_dir();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collision_after_staging_rolls_back_only_this_pairs_public_half() {
        let temp = tempfile::tempdir().unwrap();
        let out = Directory::open(&temp.path().canonicalize().unwrap(), true).unwrap();
        let mut stage = Staging::new(&out.dir).unwrap();
        let public = OsStr::new("signing.pub");
        let private = OsStr::new("signing.key");
        let identity =
            Handle::from_file(stage.write(public, b"public fixture").unwrap().into_std()).unwrap();
        stage.write(private, b"private fixture").unwrap();

        // Simulate a different process winning the second destination after we staged both halves.
        out.dir.write(private, b"another process").unwrap();
        let error = out
            .publish_pair(&stage, public, &identity, private)
            .unwrap_err();
        assert!(error.contains("signing.key"));
        assert!(!out.dir.exists(public));
        assert_eq!(out.dir.read(private).unwrap(), b"another process");
        drop(stage);
        assert_eq!(out.dir.entries().unwrap().count(), 1);

        out.dir.write(public, b"not the staged public key").unwrap();
        remove_ours(&out.dir, public, &identity).unwrap();
        assert_eq!(out.dir.read(public).unwrap(), b"not the staged public key");
    }
}
