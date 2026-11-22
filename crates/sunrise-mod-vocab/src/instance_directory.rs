//! Lossless host directory values available during registration.

/// The game's instance directory, captured before registration begins.
///
/// This is a path value, not filesystem authority. It preserves the host's encoding.
/// Never interpret Unix bytes as UTF-8 or discard unpaired Windows surrogates.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum InstanceDirectory {
    /// A Unix path, preserving every byte, including non-UTF-8 names.
    Unix(Vec<u8>),
    /// A Windows path as UTF-16 code units, preserving unpaired surrogates.
    Windows(Vec<u16>),
}

impl InstanceDirectory {
    /// The host path `path` as a lossless value; `None` on a target with neither encoding.
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Some(Self::Unix(path.as_os_str().as_bytes().to_vec()))
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            Some(Self::Windows(path.as_os_str().encode_wide().collect()))
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            None
        }
    }

    /// The host path this value names, byte for byte; `None` when it was made on another kind of
    /// host or this target supports neither encoding.
    pub fn to_path_buf(&self) -> Option<std::path::PathBuf> {
        match self {
            #[cfg(unix)]
            Self::Unix(bytes) => {
                use std::os::unix::ffi::OsStrExt;
                Some(std::ffi::OsStr::from_bytes(bytes).into())
            }
            #[cfg(not(unix))]
            Self::Unix(_) => None,
            #[cfg(windows)]
            Self::Windows(units) => {
                use std::os::windows::ffi::OsStringExt;
                Some(std::ffi::OsString::from_wide(units).into())
            }
            #[cfg(not(windows))]
            Self::Windows(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Reader, Writer};

    #[cfg(unix)]
    #[test]
    fn a_non_unicode_unix_path_round_trips_through_the_value() {
        use std::os::unix::ffi::OsStrExt;
        let path = std::path::Path::new(std::ffi::OsStr::from_bytes(b"/mods/\xff\x80/runtime"));
        let directory = InstanceDirectory::from_path(path).unwrap();
        assert_eq!(
            directory,
            InstanceDirectory::Unix(b"/mods/\xff\x80/runtime".to_vec())
        );
        assert_eq!(directory.to_path_buf().as_deref(), Some(path));
        assert_eq!(InstanceDirectory::Windows(vec![67]).to_path_buf(), None);
    }

    #[test]
    fn directory_wire_preserves_non_unicode_host_paths() {
        for directory in [
            InstanceDirectory::Unix(vec![b'/', 0xff, 0x80]),
            InstanceDirectory::Windows(vec![b'C' as u16, 58, 92, 0xd800, 0xffff]),
        ] {
            let mut output = Writer::new();
            output.put(&directory);
            let mut input = Reader::new(output.bytes());
            let decoded: InstanceDirectory = input.take().unwrap();
            assert_eq!(decoded, directory);
        }
    }
}
