//! Starling's part of native_libraries.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// What a start registers from one folder's native mod libraries: their hook entries and the build
/// records of the mods they hold.
pub struct Opened {
    pub entries: Vec<mod_api::ModEntry>,
    pub built: &'static [mod_api::__private::native::BuiltMod],
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
