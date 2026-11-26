//! Build each API-only native mod as its own Rust dylib from SDK metadata.
//! The executable is fixed and the engine archive is never read.
use super::{api, dependencies, prebuilt};
use crate::rust::Toolchain;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use sunrise_mod_package::native_route::{INTERNALS_PACKAGE, PACKAGES};

/// The template the mods library is written from, in the `api` slice.
pub const TEMPLATE: &str = "native_mod_entry.rs.template";
/// The folder of the runtime slice holding the fixed executable.
const BIN: &str = "bin";

pub(super) struct Options<'a> {
    pub on_mod: &'a dyn Fn(&str, usize, usize),
    pub check_only: bool,
}

/// Compile the mods in dependency order against API metadata and the runtime's dynamic
/// closure. Return the unchanged executable and its runtime libraries, followed by mod dylibs.
#[allow(clippy::too_many_arguments)]
pub(super) fn compile(
    rust: &dyn Toolchain,
    package: &prebuilt::Package,
    sdk: &prebuilt::Identity,
    out: &Path,
    projects: &[api::Project<'_>],
    declared: &[Vec<dependencies::Dependency>],
    resolution: &dependencies::Resolution,
    options: Options<'_>,
) -> Result<Option<(PathBuf, Vec<PathBuf>)>, String> {
    let (runtime, api) = (package.runtime.as_path(), package.api.as_path());
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let deps = api.join("deps");
    let host = api.join("host");
    let runtime_deps = runtime.join("deps");
    let search =
        |edition: &str| prebuilt::rustc_with(rust, &[&deps, &host, &runtime_deps, out], edition);
    let mut libraries = HashMap::new();
    for (package, name) in PACKAGES {
        if package != INTERNALS_PACKAGE {
            libraries.insert(name, prebuilt::library(&runtime_deps, name)?);
        }
    }
    libraries.insert("mod_vocab", prebuilt::library(&runtime_deps, "mod_vocab")?);
    let mut rlibs: HashMap<&str, PathBuf> = HashMap::new();
    let mut identities = crate::library_record::Identities::default();
    for (n, &index) in resolution.order.iter().enumerate() {
        let project = &projects[index];
        let manifest = &project.bundle.manifest;
        (options.on_mod)(&manifest.id, n + 1, resolution.order.len());
        let source = crate::source_digest(project.bundle)
            .ok_or(crate::host::tr("Could not hash native mod inputs"))?;
        let identity = identities.next(
            sdk.commit(),
            &manifest.id,
            &manifest.version.to_string(),
            &source,
            declared[index]
                .iter()
                .map(|dependency| dependency.id.as_str()),
        );
        let (rlib, _) = prebuilt::compile_mod(
            project,
            &declared[index],
            out,
            search,
            |package| &libraries[package],
            &rlibs,
            &prebuilt::library_entry(
                &api::template(&api.join(TEMPLATE))?,
                &[manifest],
                &identity,
                &[],
            ),
            &identities,
            &source,
            options.check_only,
        )?;
        rlibs.insert(project.bundle.manifest.id.as_str(), rlib);
    }
    if options.check_only {
        return Ok(None);
    }
    let mut installed = prebuilt::runtime_libraries(rust, sdk, runtime)?;
    for &index in &resolution.order {
        installed.push(rlibs[projects[index].bundle.manifest.id.as_str()].clone());
    }
    Ok(Some((runtime.join(BIN).join(crate::CLIENT), installed)))
}
