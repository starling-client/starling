//! The author-signed assets for one stable SDK. No library is loaded or extracted.
use super::{Entry, EntryVersion, Sources, read_regular, valid_repo};
use crate::index::{File, Kind, MAX_FILE, NativeBinary, Revoked, SunriseRelease, native};
use crate::sdk;
use object::{
    Architecture, BinaryFormat, Object, ObjectKind, elf,
    read::elf::{Dyn, ProgramHeader},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io::Read, path::Path};

/// Maximum bytes for a binary release descriptor.
pub const MAX_DESCRIPTOR: u64 = 256 << 10;
/// Maximum bytes for a detached minisign signature.
pub const MAX_SIGNATURE: usize = 16 << 10;
const MAX_IDENTITY: u64 = 16 << 10;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildRow {
    pub sunrise: String,
    pub binaries: String,
    pub sha256: String,
    pub signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub id: String,
    pub version: String,
    pub build: String,
    pub rust: String,
    pub archives: Vec<Archive>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    pub target: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub signature: String,
}

/// The mod's library stamp, plus the actual SDK host whose runtime it links against.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub target: String,
    pub file: String,
    pub sha256: String,
    pub identity: String,
}

/// The input the CLI supplies to check both the release's SDK and the packed source.
pub struct Checks<'a> {
    pub sdk_data: &'a Path,
    pub source: fn(&native::Bundle) -> Result<(), String>,
}

/// A hash-named input for the offline check. The descriptor has no declared size.
#[derive(Serialize)]
pub struct AssetRequest<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub sunrise: &'a str,
    pub url: String,
    pub name: String,
    pub sha256: String,
    pub size: Option<u64>,
}

pub fn descriptor_name(id: &str, version: &str, release: &str) -> String {
    format!("{id}-{version}+{release}.binaries.json")
}

pub fn archive_name(id: &str, version: &str, release: &str, target: &str) -> String {
    format!("{id}-{version}+{release}-{target}.tar.xz")
}

struct Submission<'a> {
    entry: &'a Entry,
    version: &'a EntryVersion,
    row: &'a BuildRow,
    key: &'a str,
    parent: &'a str,
    name: String,
}

fn submissions<'a>(
    entry: &'a Entry,
    version: &'a EntryVersion,
) -> Result<Vec<Submission<'a>>, String> {
    if version.builds.is_empty() {
        return Ok(Vec::new());
    }
    if entry.kind != Kind::NativeMod {
        return Err("only a native mod may list builds".into());
    }
    if version.sunrise.is_none() {
        return Err("a version with builds must name the stable release its source pins".into());
    }
    let key = entry
        .key
        .as_deref()
        .ok_or("builds need a project signing key")?;
    let source_repo = super::github_repo(&entry.source_url()).ok_or(
        "builds need the project's GitHub repository in `source`, or in `repo` when `source` is omitted",
    )?;
    let mut seen = BTreeSet::new();
    version
        .builds
        .iter()
        .map(|row| {
            let at = |e| format!("Sunrise {}: {e}", row.sunrise);
            super::check_stable_tag(&row.sunrise, &version.game).map_err(at)?;
            if !seen.insert(&row.sunrise) {
                return Err(at("build listed twice".into()));
            }
            super::hex64(&row.sha256).map_err(at)?;
            if row.signature.len() > MAX_SIGNATURE {
                return Err(at("descriptor signature is too large".into()));
            }
            let name = descriptor_name(&entry.id, &version.version, &row.sunrise);
            let invalid = || at(format!("binaries must be a GitHub release URL for {name}"));
            let (repo, tag, asset) =
                super::github_release_asset(&row.binaries).ok_or_else(invalid)?;
            if asset != name
                || !super::super::plain_name(&name)
                || !valid_repo(repo)
                || !tag.split('/').all(super::super::plain_name)
            {
                return Err(invalid());
            }
            if !repo.eq_ignore_ascii_case(&source_repo) {
                return Err(at(format!(
                    "binaries must be hosted in the project's repository {source_repo}"
                )));
            }
            Ok(Submission {
                entry,
                version,
                row,
                key,
                parent: &row.binaries[..row.binaries.len() - asset.len() - 1],
                name,
            })
        })
        .collect()
}

impl Submission<'_> {
    fn descriptor(&self, files: &Path) -> Result<Descriptor, String> {
        let bytes = read_regular(files, Path::new(&self.row.sha256), MAX_DESCRIPTOR)
            .map_err(|e| format!("{}: {e}", self.name))?;
        if crate::index::sha256(&bytes) != self.row.sha256 {
            return Err(format!("{}: descriptor SHA-256 does not match", self.name));
        }
        crate::index::verify_signature_bytes(self.key, &self.row.signature, &bytes)
            .map_err(|e| format!("{}: {e}", self.name))?;
        let descriptor: Descriptor =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", self.name))?;
        if descriptor.id != self.entry.id || descriptor.version != self.version.version {
            return Err(format!(
                "{}: descriptor id or version does not match the entry",
                self.name
            ));
        }
        if !crate::manifest::revision(&descriptor.build) {
            return Err(format!(
                "{}: descriptor build must be a full game commit",
                self.name
            ));
        }
        let mut targets = BTreeSet::new();
        for archive in &descriptor.archives {
            let expected = archive_name(
                &descriptor.id,
                &self.version.version,
                &self.row.sunrise,
                &archive.target,
            );
            if archive.name != expected
                || !super::super::plain_name(&archive.name)
                || !super::super::binary_target(&archive.target)
                || archive.size == 0
                || archive.size > MAX_FILE
                || archive.signature.len() > MAX_SIGNATURE
            {
                return Err(format!(
                    "{}: invalid archive for host {:?}",
                    self.name, archive.target
                ));
            }
            super::hex64(&archive.sha256).map_err(|e| format!("{}: {e}", archive.name))?;
            if !targets.insert(&archive.target) {
                return Err(format!("{}: repeated host {}", self.name, archive.target));
            }
        }
        Ok(descriptor)
    }
}

/// The first pass lists descriptors; the second lists archives only after binding their
/// names and hashes to each author's verified descriptor. Workflows perform the downloads.
pub fn assets<'a>(
    entries: &'a [Entry],
    files: Option<&Path>,
    revoked: &[Revoked],
) -> Result<Vec<AssetRequest<'a>>, String> {
    let mut requests = Vec::new();
    for entry in entries {
        for version in &entry.versions {
            for submission in submissions(entry, version)
                .map_err(|e| format!("{} {}: {e}", entry.id, version.version))?
            {
                if let Some(files) = files {
                    for archive in submission.descriptor(files)?.archives {
                        if revoked.iter().any(|r| r.covers(&entry.id, &archive.sha256)) {
                            continue;
                        }
                        requests.push(AssetRequest {
                            id: &entry.id,
                            version: &version.version,
                            sunrise: &submission.row.sunrise,
                            url: format!("{}/{}", submission.parent, archive.name),
                            name: archive.name,
                            sha256: archive.sha256,
                            size: Some(archive.size),
                        });
                    }
                } else {
                    requests.push(AssetRequest {
                        id: &entry.id,
                        version: &version.version,
                        sunrise: &submission.row.sunrise,
                        url: submission.row.binaries.clone(),
                        name: submission.name,
                        sha256: submission.row.sha256.clone(),
                        size: None,
                    });
                }
            }
        }
    }
    Ok(requests)
}

pub fn sdk_index(data: &Path, commit: &str) -> Result<sdk::Index, String> {
    let bytes =
        read_regular(data, &Path::new(commit).join(sdk::INDEX), MAX_DESCRIPTOR).map_err(|e| {
            format!(
                "{commit}/{}: {e}; supply the build's SDK under --sdk-data <dir>/{commit}",
                sdk::INDEX
            )
        })?;
    sdk::Index::read_for_fit(&bytes, commit).map_err(|e| e.to_string())
}

/// Use each runtime's SDK host name, including the dedicated server suffix where present.
pub fn hosts(sdk: &sdk::Index) -> Result<BTreeSet<String>, String> {
    let mut hosts = BTreeSet::new();
    for asset in sdk.assets.values() {
        if asset.kind == sdk::Kind::Runtime {
            let (target, side) = asset
                .target
                .as_deref()
                .zip(asset.side)
                .expect("validated SDK runtime");
            sdk.route_assets("api", target, side)
                .map_err(|e| e.to_string())?;
            hosts.insert(sdk::Asset::host_name(target, side));
        }
    }
    if hosts.is_empty() {
        return Err("the SDK has no runtime hosts".into());
    }
    Ok(hosts)
}

pub(super) fn verify(
    entry: &Entry,
    version: &EntryVersion,
    source: &native::Bundle,
    release: Option<&SunriseRelease>,
    src: &Sources<'_>,
    revoked: &[Revoked],
) -> Result<Vec<NativeBinary>, String> {
    let canonical = source.manifest.version.to_string();
    if version.version != canonical {
        return Err(format!(
            "builds require a canonical version label ({canonical:?}); a version already published under a short label stays source-only. Publish a new mod version to add builds"
        ));
    }
    let submissions = submissions(entry, version)?;
    let made_for = release.ok_or("builds require --releases with the source's stable release")?;
    let checks = src
        .binary_checks
        .as_ref()
        .ok_or("builds require --sdk-data and the native source validator")?;
    (checks.source)(source)?;
    let mut records = Vec::new();
    let mut commits = BTreeSet::new();
    for submission in submissions {
        let row = submission.row;
        let at = |e| format!("Sunrise {}: {e}", row.sunrise);
        let release = super::find_release(src.releases, Some(&row.sunrise))
            .map_err(at)?
            .expect("source release already resolved");
        if release.line != made_for.line {
            return Err(at(format!(
                "not on the source's release line {}",
                made_for.line
            )));
        }
        if !commits.insert(&release.commit) {
            return Err(at("a build row already targets that commit".into()));
        }
        let descriptor = submission.descriptor(src.files).map_err(at)?;
        let sdk = sdk_index(checks.sdk_data, &release.commit).map_err(at)?;
        if sdk.build.game != version.game
            || descriptor.build != release.commit
            || descriptor.rust != sdk.toolchain.rust
        {
            return Err(at(
                "descriptor build or Rust release does not match the SDK".into(),
            ));
        }
        let expected = hosts(&sdk).map_err(at)?;
        let declared = descriptor
            .archives
            .iter()
            .map(|a| a.target.clone())
            .collect::<BTreeSet<_>>();
        if declared != expected {
            let missing: Vec<_> = expected.difference(&declared).collect();
            let extra: Vec<_> = declared.difference(&expected).collect();
            return Err(at(format!(
                "must cover every SDK host: missing {missing:?}, unexpected {extra:?}"
            )));
        }
        let identity = library_identity(
            &descriptor.build,
            &descriptor.id,
            &canonical,
            &descriptor.rust,
        );
        for archive in descriptor.archives {
            if revoked.iter().any(|r| r.covers(&entry.id, &archive.sha256)) {
                continue;
            }
            let checked = read_regular(src.files, Path::new(&archive.sha256), archive.size)
                .map_err(|e| e.to_string())
                .and_then(|bytes| check_archive(bytes, &archive, &identity, submission.key));
            checked.map_err(|e| format!("{}: {e}", archive.name))?;
            records.push(NativeBinary {
                build: release.commit.clone(),
                target: archive.target,
                file: File {
                    url: format!(
                        "{}{}/{}",
                        src.release,
                        super::tag(&entry.id, &version.version),
                        archive.name
                    ),
                    name: archive.name,
                    size: archive.size,
                    sha256: archive.sha256,
                    signature: Some(archive.signature),
                },
            });
        }
    }
    Ok(records)
}

fn check_archive(
    bytes: Vec<u8>,
    archive: &Archive,
    identity: &str,
    key: &str,
) -> Result<(), String> {
    if bytes.len() as u64 != archive.size || crate::index::sha256(&bytes) != archive.sha256 {
        return Err("archive size or SHA-256 does not match the descriptor".into());
    }
    crate::index::verify_signature_bytes(key, &archive.signature, &bytes)?;
    verify_archive_identity(bytes, &archive.target, identity)
}

/// The five-field identity the builder stamps on the library and the registry checks.
pub fn library_identity(build: &str, id: &str, version: &str, rust: &str) -> String {
    format!("{build} {id} {version} {rust} unwind")
}

/// The library suffix the SDK host can load, including hosts with a `-server` suffix.
pub fn library_extension(host: &str) -> Result<&'static str, String> {
    host.split('-')
        .find_map(|part| match part {
            "darwin" => Some("dylib"),
            "linux" => Some("so"),
            "windows" => Some("dll"),
            _ => None,
        })
        .ok_or_else(|| format!("unknown native library extension for SDK host {host}"))
}

fn check_library(bytes: &[u8], host: &str, extension: &str) -> Result<(), String> {
    let format = match extension {
        "so" => BinaryFormat::Elf,
        "dll" => BinaryFormat::Pe,
        "dylib" => BinaryFormat::MachO,
        _ => unreachable!("SDK library extension"),
    };
    let arch = match host.split('-').next() {
        Some("x86_64") => Architecture::X86_64,
        Some("aarch64") => Architecture::Aarch64,
        _ => {
            return Err(format!(
                "unknown native library architecture for SDK host {host}"
            ));
        }
    };
    let invalid = || format!("SDK host {host} requires a {format:?} shared library");
    let file = object::File::parse(bytes).map_err(|e| format!("{}: {e}", invalid()))?;
    if file.format() != format || file.kind() != ObjectKind::Dynamic {
        return Err(invalid());
    }
    if file.architecture() != arch || !file.is_64() || !file.is_little_endian() {
        return Err(format!(
            "SDK host {host} requires a 64-bit little-endian {arch:?} library, found {:?}",
            file.architecture()
        ));
    }
    // ELF also calls a position-independent executable ET_DYN. A mod library must not be one.
    if let object::File::Elf64(elf) = file {
        let endian = elf.endian();
        let mut dynamic = false;
        for segment in elf.elf_program_headers() {
            if segment.p_type(endian) == elf::PT_INTERP {
                return Err(invalid());
            }
            if let Some(entries) = segment
                .dynamic(endian, bytes)
                .map_err(|e| format!("{}: {e}", invalid()))?
            {
                dynamic = !entries.is_empty();
                if entries
                    .iter()
                    .take_while(|entry| entry.tag(endian) != elf::DT_NULL)
                    .any(|entry| {
                        entry.tag(endian) == elf::DT_FLAGS_1
                            && entry.val(endian) & u64::from(elf::DF_1_PIE) != 0
                    })
                {
                    return Err(invalid());
                }
            }
        }
        if !dynamic {
            return Err(invalid());
        }
    }
    Ok(())
}

/// Check an archive's structure, encoded library and stamp without loading it. This does not
/// authenticate the bytes; an archive submitted to the registry must also pass signature checks.
pub fn verify_archive_identity(bytes: Vec<u8>, target: &str, expected: &str) -> Result<(), String> {
    let extension = library_extension(target)?;
    // Two files only; raw iteration does not allocate PAX/GNU extension entries. Bound both the
    // decoder dictionary and total output, including tar headers, zero padding and the trailer.
    let limit = MAX_FILE + MAX_IDENTITY + (1 << 16);
    let decoder = xz4rust::XzDecoder::in_heap_with_alloc_dict_size(1 << 20, sdk::DICTIONARY_LIMIT);
    let buffer = std::num::NonZeroUsize::new(1 << 16).expect("non-zero");
    let reader = xz4rust::XzReader::new_with_buffer_size_and_decoder(
        std::io::Cursor::new(bytes),
        buffer,
        decoder,
    );
    let mut tar = tar::Archive::new(reader.take(limit + 1));
    let io = |e: std::io::Error| format!("invalid tar.xz: {e}");
    let mut stamp = None;
    let mut library = None;
    for entry in tar.entries().map_err(io)?.raw(true) {
        let mut entry = entry.map_err(io)?;
        let path = entry.path_bytes();
        let name = std::str::from_utf8(&path).map_err(|_| "archive member name is not UTF-8")?;
        if !entry.header().entry_type().is_file() || !super::super::plain_name(name) {
            return Err("archive members must be regular, flat files".into());
        }
        if name == "identity.json" && stamp.is_none() && entry.size() <= MAX_IDENTITY {
            stamp = Some(
                serde_json::from_reader::<_, Identity>(&mut entry)
                    .map_err(|e| format!("identity.json: {e}"))?,
            );
        } else if library.is_none() && entry.size() > 0 && entry.size() <= MAX_FILE {
            if name
                .rsplit_once('.')
                .is_none_or(|(_, ext)| ext != extension)
            {
                return Err(format!(
                    "SDK host {target} requires a .{extension} library, not {name:?}"
                ));
            }
            let name = name.to_owned();
            let mut bytes = Vec::new();
            if entry.read_to_end(&mut bytes).map_err(io)? as u64 != entry.size() {
                return Err("truncated library".into());
            }
            check_library(&bytes, target, extension)?;
            library = Some((name, crate::index::sha256(&bytes)));
        } else {
            return Err(format!(
                "unexpected, repeated or oversized archive member {name:?}"
            ));
        }
    }
    let mut trailer = tar.into_inner();
    let mut buffer = [0; 4096];
    loop {
        let n = trailer.read(&mut buffer).map_err(io)?;
        if n == 0 {
            break;
        }
        if buffer[..n].iter().any(|b| *b != 0) {
            return Err("unexpected data after the tar members".into());
        }
    }
    if trailer.limit() == 0 {
        return Err("decoded archive exceeds its size limit".into());
    }
    let stamp = stamp.ok_or("the archive has no identity.json")?;
    let (name, sha256) = library.ok_or("the archive has no library")?;
    if stamp.target != target
        || stamp.identity != expected
        || stamp.file != name
        || stamp.sha256 != sha256
    {
        return Err(format!(
            "identity.json does not match host {target}, its library or expected identity {expected:?}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_right_extension_does_not_make_a_library_fit_the_wrong_architecture() {
        let bytes = include_bytes!(
            "../../../tests/index/stable-native/release_probe-1.0.0+26.3-0.1.1-x86_64-unknown-linux-gnu.tar.xz"
        );
        let error =
            verify_archive_identity(bytes.to_vec(), "aarch64-unknown-linux-gnu", "").unwrap_err();
        assert!(
            error.contains("Aarch64") && error.contains("X86_64"),
            "{error}"
        );
    }

    #[test]
    fn an_archive_library_must_have_its_hosts_extension() {
        let bytes = include_bytes!(
            "../../../tests/index/stable-native/release_probe-1.0.0+26.3-0.1.1-x86_64-unknown-linux-gnu.tar.xz"
        );
        for (host, extension) in [
            ("x86_64-pc-windows-gnu", "dll"),
            ("aarch64-apple-darwin", "dylib"),
        ] {
            let error = verify_archive_identity(bytes.to_vec(), host, "").unwrap_err();
            assert!(
                error.contains(&format!("SDK host {host} requires a .{extension} library")),
                "{error}"
            );
        }
        assert_eq!(
            library_extension("x86_64-unknown-linux-gnu-server").unwrap(),
            "so"
        );
    }
}
