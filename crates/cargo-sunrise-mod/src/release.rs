//! Produces archives from native-build's completed API route, not a second compiler.
use crate::{
    bounded::{self, Symlinks},
    internals::{atomic_write, check_output_dir},
};
use lzma_rust2::{XzOptions, XzWriter};
use std::{
    fs,
    path::{Path, PathBuf},
};
use sunrise_mod_package::{
    Manifest,
    index::{
        self, MAX_FILE, SunriseRelease,
        registry::{
            self,
            builds::{self, Archive, Descriptor, Identity},
        },
    },
};
use sunrise_native_build::{library_record::Library, prebuilt::Route};

mod plan;
mod prepare;

pub(crate) const USAGE: &str = "release plan|scripts|prepare-rebuild";
const SCRIPTS_USAGE: &str = "release scripts --out <new-dir>";

const SCRIPTS: &[(&str, &str)] = &[
    (
        "sunrise_release.py",
        include_str!("../template/native/sunrise_release.py"),
    ),
    (
        "sunrise_release_github.py",
        include_str!("../template/native/sunrise_release_github.py"),
    ),
];

pub(crate) fn run(project: &Path, args: &[String]) -> Result<(), String> {
    if crate::help::asked(args) {
        print!(
            "{}",
            crate::help::of("release").expect("release has a help entry")
        );
        return Ok(());
    }
    let Some((command, args)) = args.split_first() else {
        return Err(format!("use `cargo sunrise-mod {USAGE}`"));
    };
    let usage = match command.as_str() {
        "plan" => plan::USAGE,
        "scripts" => SCRIPTS_USAGE,
        "prepare-rebuild" => prepare::USAGE,
        _ => {
            return Err(format!(
                "unknown release command {command:?}; use `cargo sunrise-mod {USAGE}`"
            ));
        }
    };
    if crate::help::asked(args) {
        println!("usage: cargo sunrise-mod {usage}");
        return Ok(());
    }
    match command.as_str() {
        "plan" => return plan::run(project, args),
        "prepare-rebuild" => return prepare::run(args),
        _ => (),
    }
    let mut options =
        crate::options::Options::new("release scripts", args, &["--out"]).usage(usage);
    let values = options.values("--out")?;
    options.finish()?;
    let [out] = values.as_slice() else {
        return Err("release scripts needs one --out <new-dir>".into());
    };
    let out = output_dir(Path::new(out))?;
    let error = |error| format!("{}: {error}", out.display());
    fs::create_dir(&out).map_err(error)?;
    for (name, script) in SCRIPTS {
        fs::write(out.join(name), script).map_err(error)?;
    }
    Ok(())
}

pub(crate) fn releases(path: &Path) -> Result<Vec<SunriseRelease>, String> {
    let text = bounded::read(path, 1 << 20)?;
    registry::parse_releases(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub(crate) fn stable<'a>(
    releases: &'a [SunriseRelease],
    tag: &str,
) -> Result<&'a SunriseRelease, String> {
    releases
        .iter()
        .find(|release| release.tag == tag)
        .ok_or_else(|| format!("Sunrise {tag:?} is not in --releases"))
}

pub(crate) fn version(manifest: &Manifest) -> Result<String, String> {
    let version = manifest.version.to_string();
    if !registry::plain_version(&version) {
        return Err(format!(
            "version {version:?} cannot name a registry release: use SemVer without +build metadata and at most 64 characters"
        ));
    }
    Ok(version)
}

/// Read a detached signature without following a linked sidecar or treating arbitrary text as one.
pub(crate) fn signature(path: &Path) -> Result<String, String> {
    let bytes = bounded::read_bytes(path, builds::MAX_SIGNATURE as u64, Symlinks::Refuse)
        .map_err(|error| error.to_string())?;
    let invalid =
        |error: &dyn std::fmt::Display| format!("{}: invalid .minisig: {error}", path.display());
    let text = String::from_utf8(bytes).map_err(|error| invalid(&error))?;
    minisign_verify::Signature::decode(&text).map_err(|error| invalid(&error))?;
    Ok(text)
}

/// A generated filename that the registry accepts as one release asset name.
pub(crate) struct AssetName(String);

impl AssetName {
    pub(crate) fn archive(
        id: &str,
        version: &str,
        tag: &str,
        target: &str,
    ) -> Result<Self, String> {
        Self::checked(builds::archive_name(id, version, tag, target))
    }

    pub(crate) fn descriptor(id: &str, version: &str, tag: &str) -> Result<Self, String> {
        Self::checked(builds::descriptor_name(id, version, tag))
    }

    fn checked(name: String) -> Result<Self, String> {
        if !index::plain_name(&name) {
            return Err(format!(
                "{name}: a release asset name must be plain and at most 128 bytes; shorten the mod id or version"
            ));
        }
        Ok(Self(name))
    }
}

pub(crate) struct BuiltLibrary {
    path: PathBuf,
    stamp: Identity,
}

impl BuiltLibrary {
    pub(crate) fn from_event(
        route: &str,
        host: String,
        path: &Path,
        record: &Library,
        identity: &str,
    ) -> Result<Self, String> {
        if route != Route::Api.name() {
            return Err(format!(
                "the build used {route}, not {}: no independent library can be released",
                Route::Api.name()
            ));
        }
        let extension = builds::library_extension(&host)?;
        if path.extension().and_then(|suffix| suffix.to_str()) != Some(extension) {
            return Err(format!(
                "SDK host {host} requires a .{extension} library, not {:?}",
                record.file
            ));
        }
        if record.identity != identity {
            return Err(format!(
                "{} was built with identity {:?}, expected {identity:?}",
                path.display(),
                record.identity
            ));
        }
        Ok(Self {
            path: path.to_owned(),
            stamp: Identity {
                target: host,
                file: record.file.clone(),
                sha256: record.sha256.clone(),
                identity: record.identity.clone(),
            },
        })
    }

    pub(crate) fn write(&self, dir: &Path, name: AssetName) -> Result<PathBuf, String> {
        let out = dir.join(name.0);
        let bytes = bounded::read_bytes(&self.path, MAX_FILE, Symlinks::Refuse)
            .map_err(|e| e.to_string())?;
        if bytes.is_empty() || index::sha256(&bytes) != self.stamp.sha256 {
            return Err(format!(
                "{} no longer matches the completed build's library record",
                self.path.display()
            ));
        }
        let stamp = serde_json::to_vec(&self.stamp).map_err(|e| e.to_string())?;
        let archive = archive_bytes(&self.stamp.file, &bytes, &stamp)
            .map_err(|e| format!("{}: {e}", out.display()))?;
        if archive.len() as u64 > MAX_FILE {
            return Err(format!(
                "{} exceeds the registry's archive size limit",
                out.display()
            ));
        }
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        atomic_write(&out, archive)?;
        Ok(out)
    }
}

fn archive_bytes(name: &str, library: &[u8], stamp: &[u8]) -> std::io::Result<Vec<u8>> {
    let xz = XzWriter::new(Vec::new(), XzOptions::with_preset(3))?;
    let mut tar = tar::Builder::new(xz);
    for (name, bytes) in [(name, library), ("identity.json", stamp)] {
        let mut header = tar::Header::new_ustar();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        header.set_size(bytes.len() as u64);
        tar.append_data(&mut header, name, bytes)?;
    }
    tar.into_inner()?.finish()
}

pub(crate) fn output_dir(dir: &Path) -> Result<PathBuf, String> {
    let dir: PathBuf = dir.components().collect();
    if dir.as_os_str().is_empty() {
        return Err("name a folder for the release assets".into());
    }
    for part in dir.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        check_output_dir(part)?;
    }
    Ok(dir)
}

/// Describe all signed archives on the SDK host matrix. This step does not compile or sign.
pub(crate) fn describe(
    manifest: &Manifest,
    dir: &Path,
    release: &SunriseRelease,
    sdk_data: &Path,
) -> Result<PathBuf, String> {
    let native = manifest
        .native
        .as_ref()
        .ok_or("build --describe-binaries needs a native mod")?;
    let version = version(manifest)?;
    let name = AssetName::descriptor(&manifest.id, &version, &release.tag)?;
    let sdk = builds::sdk_index(sdk_data, &release.commit)?;
    if sdk.build.game != native.game {
        return Err(format!(
            "Sunrise {} has Minecraft {}, the mod is for {}",
            release.tag, sdk.build.game, native.game
        ));
    }
    let mut descriptor = Descriptor {
        id: manifest.id.clone(),
        version,
        build: release.commit.clone(),
        rust: sdk.toolchain.rust.clone(),
        archives: Vec::new(),
    };
    let identity = builds::library_identity(
        &descriptor.build,
        &descriptor.id,
        &descriptor.version,
        &descriptor.rust,
    );
    let mut names = std::collections::BTreeSet::new();
    for target in builds::hosts(&sdk)? {
        let AssetName(name) =
            AssetName::archive(&descriptor.id, &descriptor.version, &release.tag, &target)?;
        let path = dir.join(&name);
        let bytes =
            bounded::read_bytes(&path, MAX_FILE, Symlinks::Refuse).map_err(|e| e.to_string())?;
        let size = bytes.len() as u64;
        let sha256 = index::sha256(&bytes);
        let signature = signature(&dir.join(format!("{name}.minisig")))?;
        builds::verify_archive_identity(bytes, &target, &identity)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        names.insert(name.clone());
        descriptor.archives.push(Archive {
            target,
            name,
            size,
            sha256,
            signature,
        });
    }
    let prefix = format!("{}-{}+{}-", descriptor.id, descriptor.version, release.tag);
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        if let Some(name) = entry.file_name().to_str()
            && name.starts_with(&prefix)
            && name.ends_with(".tar.xz")
            && !names.contains(name)
        {
            return Err(format!(
                "{name}: not an archive for a host in the release SDK"
            ));
        }
    }
    let path = dir.join(name.0);
    let mut bytes = serde_json::to_vec_pretty(&descriptor).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if bytes.len() as u64 > builds::MAX_DESCRIPTOR {
        return Err(format!(
            "{} exceeds the registry's descriptor size limit",
            path.display()
        ));
    }
    atomic_write(&path, bytes)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_scripts_are_written_exactly_and_do_not_replace_existing_files() {
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("scripts");
        let args = [
            "scripts".to_owned(),
            "--out".to_owned(),
            out.display().to_string(),
        ];
        run(temp.path(), &args).unwrap();
        assert_eq!(fs::read_dir(&out).unwrap().count(), 2);
        for (name, bytes) in [
            (
                "sunrise_release.py",
                include_str!("../template/native/sunrise_release.py"),
            ),
            (
                "sunrise_release_github.py",
                include_str!("../template/native/sunrise_release_github.py"),
            ),
        ] {
            assert_eq!(fs::read(out.join(name)).unwrap(), bytes.as_bytes());
        }
        let keep = out.join("sunrise_release.py");
        fs::write(&keep, b"keep these bytes").unwrap();
        assert!(run(temp.path(), &args).is_err());
        assert_eq!(fs::read(keep).unwrap(), b"keep these bytes");
    }

    #[test]
    fn archive_from_the_completed_api_library_passes_the_registry_decoder() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = include_bytes!(
            "../../sunrise-mod-package/tests/index/stable-native/release_probe-1.0.0+26.3-0.1.1-x86_64-unknown-linux-gnu.tar.xz"
        );
        let mut tar = tar::Archive::new(lzma_rust2::XzReader::new(fixture.as_slice(), false));
        let mut library = Vec::new();
        for entry in tar.entries().unwrap() {
            let mut entry = entry.unwrap();
            if entry.path_bytes().ends_with(b".so") {
                std::io::Read::read_to_end(&mut entry, &mut library).unwrap();
            }
        }
        let id = "probe";
        let version = "1.2.0";
        let host = "x86_64-unknown-linux-gnu-server";
        let identity = format!("{} probe 1.2.0 1.99.1 unwind", "a".repeat(40));
        let record = Library {
            file: "libmod_probe.so".into(),
            sha256: index::sha256(&library),
            kind: "mod".into(),
            mod_id: id.into(),
            identity: identity.clone(),
        };
        let path = temp.path().join(&record.file);
        fs::write(&path, library).unwrap();
        let built =
            BuiltLibrary::from_event(Route::Api.name(), host.into(), &path, &record, &identity)
                .unwrap();
        let archive = built
            .write(
                temp.path(),
                AssetName::archive(id, version, "26.3-0.1.0", host).unwrap(),
            )
            .unwrap();
        assert_eq!(
            archive.file_name().unwrap(),
            "probe-1.2.0+26.3-0.1.0-x86_64-unknown-linux-gnu-server.tar.xz"
        );
        builds::verify_archive_identity(fs::read(&archive).unwrap(), host, &identity).unwrap();
        fs::write(path, b"not the recorded library").unwrap();
        assert!(
            built
                .write(
                    temp.path(),
                    AssetName::archive(id, version, "26.3-0.1.1", host).unwrap()
                )
                .unwrap_err()
                .contains("no longer matches")
        );
    }

    #[test]
    fn overlong_release_assets_are_refused_before_compilation_or_description() {
        let id = "m".repeat(64);
        let version = format!("1.0.0-{}", "a".repeat(37));
        let tag = "26.3-0.1.0";
        assert!(index::plain_name(&format!("{id}-{version}.smod")));
        let work = tempfile::tempdir().unwrap();
        let manifest = Manifest::parse(&format!(
            "id='{id}'\nname='Probe'\nversion='{version}'\n[native]\ngame='26.3'\nsunrise='{}'\n",
            "a".repeat(40)
        ))
        .unwrap();
        let release = SunriseRelease {
            tag: tag.into(),
            commit: "a".repeat(40),
            line: "26.3-0".into(),
        };
        let out = work.path().join("assets");
        let error = describe(&manifest, &out, &release, work.path()).unwrap_err();
        assert!(error.contains("128 bytes"), "{error}");
        let error = AssetName::archive(&id, &version, tag, "x86_64-unknown-linux-gnu-server")
            .err()
            .unwrap();
        assert!(error.contains("128 bytes"), "{error}");
        assert!(!out.exists());
    }

    #[test]
    fn release_library_rejects_the_wrong_route_host_file_or_build_stamp() {
        let record = Library {
            file: "mod_probe.so".into(),
            sha256: "0".repeat(64),
            kind: "mod".into(),
            mod_id: "probe".into(),
            identity: "expected".into(),
        };
        let path = Path::new(&record.file);
        for (route, host, identity, error) in [
            (
                Route::Engine,
                "x86_64-unknown-linux-gnu",
                "expected",
                "not prebuilt-api",
            ),
            (
                Route::Api,
                "x86_64-pc-windows-gnu",
                "expected",
                "requires a .dll",
            ),
            (
                Route::Api,
                "x86_64-unknown-linux-gnu",
                "another build",
                "expected \"another build\"",
            ),
        ] {
            let result =
                BuiltLibrary::from_event(route.name(), host.into(), path, &record, identity);
            assert!(result.err().unwrap().contains(error));
        }
    }
}
