//! A package's runtime data files, read through its registration context.
//!
//! A mod's package may carry a `runtime/` folder of data its code opens beside itself (a
//! compiled script bundle, a baked lookup table), each file listed with its SHA-512 in the
//! manifest's `[runtime]` table. The game checks every hash once, when the package opens, and
//! serves those bytes: a read never hashes again.
use std::{path::Path, sync::Arc};

/// The game's side of one package's runtime files. A mod reaches it through
/// [`RegistrationContext::runtime_files`](crate::RegistrationContext::runtime_files), not
/// directly.
pub trait RuntimeFileSource: Send + Sync + 'static {
    /// Borrow the listed names in name order.
    fn names(&self) -> &[String];
    /// Borrow the checked bytes of `name`; `None` for a name the manifest does not list.
    fn file(&self, name: &str) -> Option<&[u8]>;
    /// Borrow the exact live disk path, without converting through UTF-8. Return
    /// [`RuntimePathError::NotListed`] for an unlisted name or [`RuntimePathError::NotAFolder`]
    /// for a listed file in a `.smod` archive. Look up the name directly rather than scanning.
    fn path(&self, name: &str) -> Result<&Path, RuntimePathError>;
}

/// A package's runtime files were asked for and are not there.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum RuntimeFilesError {
    /// The requested package declares no `[runtime]`, is not loaded, or no registration
    /// snapshot is installed.
    NoRuntimeFiles,
}
impl std::fmt::Display for RuntimeFilesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRuntimeFiles => f.write_str("package declares no [runtime] files"),
        }
    }
}
impl std::error::Error for RuntimeFilesError {}

/// A path read of a runtime file was refused.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum RuntimePathError {
    /// The package does not list this name.
    NotListed,
    /// The package is a `.smod` archive, which has no file to point at; read the bytes.
    NotAFolder,
}
impl std::fmt::Display for RuntimePathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotListed => "the package's [runtime] does not list this file",
            Self::NotAFolder => {
                "the package is a .smod archive, so its runtime files have no path: read the bytes"
            }
        })
    }
}
impl std::error::Error for RuntimePathError {}

/// A package's runtime files, from
/// [`RegistrationContext::runtime_files`](crate::RegistrationContext::runtime_files). A mod may
/// keep it and read from any thread. Reads borrow from this handle without copying. The bytes were
/// checked against the manifest's SHA-512 when the package opened, and no read checks them again.
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: a native handle retaining the game's verified file source.
#[derive(Clone)]
pub struct RuntimeFiles {
    source: Arc<dyn RuntimeFileSource>,
}

impl RuntimeFiles {
    /// The names of the files the package lists, borrowed in name order.
    pub fn names(&self) -> &[String] {
        self.source.names()
    }

    /// The verified bytes of `name`, borrowed without copying. Use the name the manifest lists
    /// under `[runtime]`, without a `runtime/` prefix. Returns `None` for any other name.
    pub fn file(&self, name: &str) -> Option<&[u8]> {
        self.source.file(name)
    }

    /// The exact host path to a listed file in a package folder, borrowed without copying.
    /// Returns [`RuntimePathError::NotListed`] for an unlisted name and
    /// [`RuntimePathError::NotAFolder`] for a listed file in a `.smod` archive.
    ///
    /// This path refers to the live file on disk, not to the verified byte snapshot. Use
    /// [`Self::file`] to read the bytes checked when the package opened.
    pub fn file_path(&self, name: &str) -> Result<&Path, RuntimePathError> {
        self.source.path(name)
    }
}

impl crate::RegistrationContext {
    /// The runtime files of `mod_id`, from its package's `[runtime]`. Available from registration
    /// on, before any level exists; keep the handle to read later.
    /// Returns [`RuntimeFilesError::NoRuntimeFiles`] when the package declares none, no package
    /// with that ID is loaded, or no registration snapshot is installed.
    pub fn runtime_files(&self, mod_id: &str) -> Result<RuntimeFiles, RuntimeFilesError> {
        self.data
            .as_ref()
            .and_then(|data| data.runtime_source(mod_id))
            .map(|source| RuntimeFiles { source })
            .ok_or(RuntimeFilesError::NoRuntimeFiles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Fake {
        names: [String; 1],
        bytes: [u8; 6],
        path: Option<PathBuf>,
    }

    impl Fake {
        fn new(path: Option<PathBuf>) -> Self {
            Self {
                names: ["nav.bin".into()],
                bytes: *b"tables",
                path,
            }
        }
    }

    impl RuntimeFileSource for Fake {
        fn names(&self) -> &[String] {
            &self.names
        }
        fn file(&self, name: &str) -> Option<&[u8]> {
            (name == "nav.bin").then_some(self.bytes.as_slice())
        }
        fn path(&self, name: &str) -> Result<&Path, RuntimePathError> {
            if name != "nav.bin" {
                return Err(RuntimePathError::NotListed);
            }
            self.path.as_deref().ok_or(RuntimePathError::NotAFolder)
        }
    }

    #[test]
    fn a_mod_borrows_only_the_names_and_bytes_its_manifest_lists() {
        let source = Arc::new(Fake::new(None));
        let files = RuntimeFiles {
            source: source.clone(),
        };
        assert_eq!(files.names(), ["nav.bin"]);
        assert!(std::ptr::eq(files.names(), source.names()));
        let bytes = files.file("nav.bin").unwrap();
        assert_eq!(bytes, b"tables");
        assert!(std::ptr::eq(bytes, source.bytes.as_slice()));
        assert_eq!(files.file("other.bin"), None);
        assert_eq!(files.file("../nav.bin"), None);
    }

    #[test]
    fn runtime_files_looks_up_any_mod_by_id_without_caller_binding() {
        let mut context = crate::RegistrationContext::new();
        assert_eq!(
            context.runtime_files("reader_a").err(),
            Some(RuntimeFilesError::NoRuntimeFiles)
        );
        let source = Arc::new(Fake::new(None));
        let data = crate::world::phase::RegistrationData::new(
            Default::default(),
            Default::default(),
            crate::InstanceDirectory::Unix(b"/game".to_vec()),
            crate::RuntimeEnvironment::new(
                crate::Dist::Client,
                "26.3".into(),
                "test".into(),
                false,
            ),
        )
        .with_runtime_files(std::collections::BTreeMap::from([(
            "reader_a".to_owned(),
            source.clone() as Arc<dyn RuntimeFileSource>,
        )]));
        context.data = Some(data);

        let files = context.runtime_files("reader_a").unwrap();
        assert!(std::ptr::eq(files.names(), source.names()));
        assert_eq!(files.file("nav.bin"), Some(b"tables".as_slice()));
        assert_eq!(
            context.runtime_files("reader_b").err(),
            Some(RuntimeFilesError::NoRuntimeFiles)
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn a_mod_borrows_the_exact_host_path() {
        #[cfg(unix)]
        let expected = {
            use std::os::unix::ffi::OsStrExt;
            PathBuf::from(std::ffi::OsStr::from_bytes(
                b"/mods/\xff ruby/runtime/nav.bin",
            ))
        };
        #[cfg(windows)]
        let expected = {
            use std::os::windows::ffi::OsStringExt;
            let mut units: Vec<u16> = "C:\\mods\\".encode_utf16().collect();
            units.push(0xd800);
            units.extend("ruby\\runtime\\nav.bin".encode_utf16());
            PathBuf::from(std::ffi::OsString::from_wide(&units))
        };
        let source = Arc::new(Fake::new(Some(expected.clone())));
        let folder = RuntimeFiles {
            source: source.clone(),
        };
        let path = folder.file_path("nav.bin").unwrap();
        assert_eq!(path.as_os_str(), expected.as_os_str());
        assert!(std::ptr::eq(path, source.path.as_deref().unwrap()));
    }
}
