//! Checked, process-lifetime native library loading.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{ffi::CStr, path::Path};

/// Commit used for the pinned SDK ABI; `dev` for a build that names none.
pub const GAME_BUILD_ID: &str = env!("SUNRISE_GAME_BUILD_ID");
/// Compiler release used for the pinned Rust ABI.
pub const RUSTC_VERSION: &str = env!("SUNRISE_RUSTC_VERSION");

#[derive(Deserialize)]
struct Record {
    libraries: Vec<Library>,
}
#[derive(Deserialize)]
struct Library {
    file: String,
    sha256: String,
    #[serde(rename = "mod")]
    id: String,
    identity: String,
}

/// A build identity is `<build id> <mod id> <version> <rustc release> <panic strategy>`.
/// Returns the build id, the rustc release and the panic strategy.
fn identity_parts(identity: &str) -> Option<(&str, &str, &str)> {
    let parts: Vec<_> = identity.split_whitespace().collect();
    (parts.len() == 5).then(|| (parts[0], parts[3], parts[4]))
}

/// The shared libraries an image needs: the dynamic section's `DT_NEEDED` entries on ELF, the
/// imported libraries on Mach-O and PE.
fn needed_libraries(bytes: &[u8]) -> Result<Vec<String>, String> {
    use object::{
        Object,
        read::elf::{Dyn, ElfFile64},
    };
    let file = object::File::parse(bytes).map_err(|e| e.to_string())?;
    if let object::File::Elf64(_) = &file {
        let elf = ElfFile64::<object::Endianness>::parse(bytes).map_err(|e| e.to_string())?;
        let endian = elf.endian();
        let Some((entries, link)) = elf
            .elf_section_table()
            .dynamic(endian, bytes)
            .map_err(|e| e.to_string())?
        else {
            return Ok(Vec::new());
        };
        let strings = elf
            .elf_section_table()
            .strings(endian, bytes, link)
            .map_err(|e| e.to_string())?;
        return entries
            .iter()
            .filter(|entry| entry.tag32(endian) == Some(object::elf::DT_NEEDED as i32))
            .map(|entry| {
                let name = entry.string(endian, strings).map_err(|e| e.to_string())?;
                Ok(String::from_utf8_lossy(name).into_owned())
            })
            .collect();
    }
    Ok(file
        .imports()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|import| String::from_utf8_lossy(import.library()).into_owned())
        .collect())
}

/// The libstd image an image needs (`libstd-<hash>.so`, `std-<hash>.dll`), if any.
fn std_library(needed: &[String]) -> Option<&str> {
    needed.iter().map(String::as_str).find(|name| {
        let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
        name.strip_prefix("lib").unwrap_or(name).starts_with("std-")
    })
}

/// The Rust allocator shim a `#[global_allocator]` defines (`__rust_alloc` and its family,
/// whatever the platform or symbol version prefixes it with). The game's std provides them and
/// every image binds to that one.
const ALLOCATOR_SHIM: [&str; 3] = ["__rust_alloc", "__rust_dealloc", "__rust_realloc"];

/// Refuses a library before any of its code runs: it must unwind, use the game's libstd and
/// define no allocator.
fn check_image(id: &str, bytes: &[u8], game_std: Option<&str>) -> Result<(), String> {
    use object::Object;
    let needed =
        needed_libraries(bytes).map_err(|e| format!("mod {id}: unreadable library: {e}"))?;
    // Starling's Android game builds its std into libsunrise_core (build-std, no libstd-<hash>.so):
    // there the game and its mods need none, and share the one in libsunrise_core.
    let shared_in_core = cfg!(target_os = "android") && game_std.is_none();
    if std_library(&needed) != game_std || (game_std.is_none() && !shared_in_core) {
        return Err(format!(
            "mod {id}: its libstd is {:?}, not the game's {game_std:?}: build the mod against this game's SDK, which shares one std",
            std_library(&needed)
        ));
    }
    let file =
        object::File::parse(bytes).map_err(|e| format!("mod {id}: unreadable library: {e}"))?;
    let exports = file
        .exports()
        .map_err(|e| format!("mod {id}: unreadable library: {e}"))?;
    if let Some(shim) = exports.iter().find_map(|export| {
        let name = String::from_utf8_lossy(export.name());
        ALLOCATOR_SHIM
            .iter()
            .any(|shim| name.contains(shim))
            .then(|| name.into_owned())
    }) {
        return Err(format!(
            "mod {id}: the library defines the allocator symbol {shim}, so it has its own #[global_allocator]; the game's allocator serves every mod, remove it"
        ));
    }
    Ok(())
}

/// Checks each record row against the installation and returns the bytes the hash covers.
fn verify(folder: &Path, libraries: &[Library]) -> Result<Vec<Vec<u8>>, String> {
    let mut images = Vec::new();
    for library in libraries {
        if identity_parts(&library.identity)
            .is_none_or(|(build, rustc, _)| build != GAME_BUILD_ID || rustc != RUSTC_VERSION)
        {
            return Err(format!(
                "mod {}: incompatible build identity {:?}; game {GAME_BUILD_ID} rustc {RUSTC_VERSION}. The library was built for another game build or Rust release: rebuild the mod for this build (`cargo sunrise-mod update --sunrise {GAME_BUILD_ID}`, then `cargo sunrise-mod build`)",
                library.id, library.identity
            ));
        }
        // File names are untrusted build-record input, not paths outside this installation.
        if Path::new(&library.file).components().count() != 1
            || !matches!(
                Path::new(&library.file).components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Err(format!(
                "mod {}: invalid library file {}",
                library.id, library.file
            ));
        }
        // The launcher installs real files (copies or hard links), never symlinks: a link could
        // lead out of the installation.
        let path = folder.join(&library.file);
        let is_link = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("mod {}: {}: {error}", library.id, library.file))?
            .file_type()
            .is_symlink();
        if is_link {
            return Err(format!(
                "mod {}: library file {} is a symbolic link",
                library.id, library.file
            ));
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("mod {}: {}: {error}", library.id, library.file))?;
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if actual != library.sha256 {
            return Err(format!(
                "mod {}: library {} hash mismatch (expected {}, found {actual})",
                library.id, library.file, library.sha256
            ));
        }
        images.push(bytes);
    }
    Ok(images)
}

/// Checks and loads every native library `folder`'s `native-mods.json` lists, refusing by mod name.
/// Public so the daily linkage probe (`tools/native_dylib_probe`) goes through the loader the game uses.
pub fn load(folder: &Path) -> Result<Vec<&'static mod_api::__private::native::NativeMod>, String> {
    let path = folder.join("native-mods.json");
    let record = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Record>(&bytes)
            .map_err(|e| format!("native-mods.json: {e}"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("native-mods.json: {e}")),
    };
    let exe = std::fs::read(std::env::current_exe().map_err(|e| e.to_string())?)
        .map_err(|e| format!("the game executable: {e}"))?;
    let game_needed = needed_libraries(&exe).map_err(|e| format!("the game executable: {e}"))?;
    let game_std = std_library(&game_needed);
    // Every image is read and refused before any of them is loaded: no code of a refused library runs.
    for (row, image) in record
        .libraries
        .iter()
        .zip(verify(folder, &record.libraries)?)
    {
        check_image(&row.id, &image, game_std)?;
    }
    let mut loaded = Vec::new();
    for row in &record.libraries {
        let path = folder.join(&row.file);
        // SAFETY: loading native code is an explicit trust boundary. Keep every handle forever,
        // including on refusal, so returned Rust references and function pointers never dangle.
        let library: libloading::Library = unsafe {
            #[cfg(windows)]
            {
                libloading::os::windows::Library::load_with_flags(&path, 0x100 | 0x1000)
                    .map(Into::into)
            }
            #[cfg(not(windows))]
            {
                libloading::Library::new(&path)
            }
        }
        .map_err(|e| format!("mod {}: {e}", row.id))?;
        let library = Box::leak(Box::new(library));
        unsafe {
            let build = library
                .get::<unsafe extern "C" fn() -> *const core::ffi::c_char>(
                    b"sunrise_native_build\0",
                )
                .map_err(|e| format!("mod {}: {e}", row.id))?;
            let pointer = build();
            if pointer.is_null() {
                return Err(format!("mod {}: null build identity", row.id));
            }
            let identity = CStr::from_ptr(pointer)
                .to_str()
                .map_err(|e| format!("mod {}: {e}", row.id))?;
            if identity_parts(identity).is_some_and(|(_, _, panic)| panic != "unwind") {
                return Err(format!(
                    "mod {}: built with panic=abort ({identity:?}); the game unwinds a refused Host call through the mod's frames, so a native mod must be built with panic=unwind",
                    row.id
                ));
            }
            if identity != row.identity
                || identity_parts(identity).is_none_or(|(build, rustc, _)| {
                    build != GAME_BUILD_ID || rustc != RUSTC_VERSION
                })
            {
                return Err(format!(
                    "mod {}: identity {identity:?}, expected {:?}; game {GAME_BUILD_ID} rustc {RUSTC_VERSION}",
                    row.id, row.identity
                ));
            }
        }
        loaded.push((row, library));
    }
    // No registration factory is called until every library has passed identity validation.
    loaded
        .into_iter()
        .map(|(row, library)| unsafe {
            let entry = library
                .get::<unsafe fn() -> &'static mod_api::__private::native::NativeMod>(
                    b"sunrise_native_mod_v1\0",
                )
                .map_err(|e| format!("mod {}: {e}", row.id))?;
            Ok(entry())
        })
        .collect()
}

/// What a start registers from one folder's native mod libraries: their hook entries and the build
/// records of the mods they hold.
pub struct Opened {
    pub entries: Vec<mod_api::ModEntry>,
    pub built: &'static [mod_api::__private::native::BuiltMod],
}

/// The libraries `folder`'s `native-mods.json` lists (none without one), checked, opened and
/// installed. Every patch copy is installed and checked before any mod's hook factory runs, as the
/// generated `main` did: a factory may call a patched function, and a refused install stops the
/// start before any mod code beyond the installs has run. The desktop's fixed executable reads the
/// folder beside it ([`main`]); Starling's Android start reads one in the app's own storage
/// (android_lib.rs), since a phone has no executable to put them beside.
pub fn open(folder: &Path) -> Result<Opened, String> {
    let libraries = load(folder)?;
    let mut installs = Vec::new();
    for library in &libraries {
        installs.extend((library.installs)());
    }
    sunrise_client::entry::installed(&installs);
    let mut entries = Vec::new();
    let mut built = Vec::new();
    for library in libraries {
        entries.extend((library.entries)());
        built.extend_from_slice(library.built);
    }
    Ok(Opened {
        entries,
        built: Box::leak(built.into_boxed_slice()),
    })
}

/// The packages of the mods `built` names, read where [`main_with`](crate::main_with) reads them
/// beside an executable: `folder`'s `sunrise-native-mods/<id>/`. For a start that has no
/// executable beside its libraries (Starling's Android start).
pub fn packages_beside(
    folder: &Path,
    built: &'static [mod_api::__private::native::BuiltMod],
) -> Result<sunrise_mod_package::runtime::Runtime, String> {
    // `Runtime::load` reads the folder named `sunrise-native-mods` beside the file it is given.
    sunrise_mod_package::runtime::Runtime::load(&folder.join("game"), built)
        .map_err(|error| error.to_string())
}

/// Start the fixed executable, registering checked native libraries before the engine seal.
pub fn main() {
    let startup = sunrise_client::entry::prepare();
    let result = std::env::current_exe()
        .map_err(|e| e.to_string())
        .and_then(|exe| open(exe.parent().expect("executable parent")));
    let opened = result.unwrap_or_else(|error| {
        eprintln!("Sunrise: refusing native startup: {error}");
        std::process::exit(1);
    });
    crate::main_with(opened.entries, opened.built, startup);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_phase_refuses_missing_corrupt_and_traversal_by_mod_name() {
        let folder = std::env::temp_dir().join(format!("native-hash-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let mut row = Library {
            file: "test.so".into(),
            sha256: format!("{:x}", Sha256::digest(b"valid")),
            id: "example".into(),
            identity: format!("{GAME_BUILD_ID} example 1 {RUSTC_VERSION} unwind"),
        };
        assert!(
            verify(&folder, std::slice::from_ref(&row))
                .unwrap_err()
                .contains("example")
        );
        std::fs::write(folder.join("test.so"), b"valid").unwrap();
        verify(&folder, std::slice::from_ref(&row)).unwrap();
        for identity in [
            format!("wrong example 1 {RUSTC_VERSION} unwind"),
            format!("{GAME_BUILD_ID} example 1 wrong unwind"),
        ] {
            let valid = std::mem::replace(&mut row.identity, identity);
            let error = verify(&folder, std::slice::from_ref(&row)).unwrap_err();
            assert!(error.contains("example") && error.contains("identity"));
            row.identity = valid;
        }
        assert!(serde_json::from_str::<Record>("{}").is_err());
        assert!(serde_json::from_str::<Record>(r#"{"libraries": []}"#).is_ok());
        std::fs::write(folder.join("test.so"), b"corrupt").unwrap();
        assert!(
            verify(&folder, std::slice::from_ref(&row))
                .unwrap_err()
                .contains("example")
        );
        // A link with the right bytes is still refused: it could lead out of the installation.
        #[cfg(unix)]
        {
            std::fs::write(folder.join("real.so"), b"valid").unwrap();
            std::os::unix::fs::symlink(folder.join("real.so"), folder.join("link.so")).unwrap();
            row.file = "link.so".into();
            let error = verify(&folder, std::slice::from_ref(&row)).unwrap_err();
            assert!(
                error.contains("example") && error.contains("symbolic link"),
                "{error}"
            );
        }
        row.file = "../test.so".into();
        assert!(verify(&folder, &[row]).unwrap_err().contains("example"));
        std::fs::remove_dir_all(folder).unwrap();
    }
}
