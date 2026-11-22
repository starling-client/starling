//! Read a signed release folder using the same verifier as `registry check`.
use crate::{
    bounded::{self, Symlinks},
    registry::fit,
    release,
};
use std::{fs, path::Path};
use sunrise_mod_package::index::{
    self, SunriseRelease,
    registry::{
        self, Entry, EntryVersion, Sources,
        builds::{self, BuildRow},
    },
};

pub(super) struct Asset {
    pub name: String,
    pub sha256: String,
}

pub(super) struct Submission {
    pub build: BuildRow,
    pub descriptor_signature_name: String,
    pub assets: Vec<Asset>,
    pub verified_files: tempfile::TempDir,
}

pub(super) struct Inputs<'a> {
    pub dir: &'a Path,
    pub base: &'a str,
    pub releases: &'a [SunriseRelease],
    pub sdk_data: &'a Path,
}

impl Inputs<'_> {
    pub fn prepare(
        &self,
        entry: &Entry,
        version: &EntryVersion,
        source: &[u8],
    ) -> Result<Submission, String> {
        let candidates = self
            .releases
            .iter()
            .filter_map(|release| {
                let name = builds::descriptor_name(&entry.id, &version.version, &release.tag);
                match fs::symlink_metadata(self.dir.join(&name)) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => Some(Err(error.to_string())),
                    Ok(_) => Some(Ok((release, name))),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let [(release, name)] = candidates.as_slice() else {
            return Err(format!(
                "{} must hold one signed descriptor for {} {} and a tag in --releases",
                self.dir.display(),
                entry.id,
                version.version
            ));
        };
        for old in &version.builds {
            if release.commit == release::stable(self.releases, &old.sunrise)?.commit {
                return Err(format!(
                    "{} {} already has the build for {}; published builds are append-only",
                    entry.id, version.version, old.sunrise
                ));
            }
        }
        let bytes = bounded::read_bytes(
            &self.dir.join(name),
            builds::MAX_DESCRIPTOR,
            Symlinks::Refuse,
        )
        .map_err(|e| e.to_string())?;
        let descriptor_signature_name = format!("{name}.minisig");
        let signature = release::signature(&self.dir.join(&descriptor_signature_name))?;
        let hash = index::sha256(&bytes);
        let build = BuildRow {
            sunrise: release.tag.clone(),
            binaries: format!("{}/{}", self.base, name),
            sha256: hash.clone(),
            signature,
        };
        // The verifier's input is hash-named. Use a private copy of each input so the files
        // that pass verification are exactly the bytes the submission will write.
        let scratch = tempfile::tempdir().map_err(|e| e.to_string())?;
        let put = |name: &str, bytes: &[u8]| {
            fs::write(scratch.path().join(name), bytes).map_err(|e| e.to_string())
        };
        put(&hash, &bytes)?;
        let mut assets = vec![Asset {
            name: name.clone(),
            sha256: hash,
        }];
        let mut checked = entry.clone();
        let mut candidate = version.clone();
        candidate.builds = vec![build.clone()];
        checked.versions = vec![candidate];
        // The second pass lists only archives; a signed descriptor supplies each size.
        for request in builds::assets(std::slice::from_ref(&checked), Some(scratch.path()), &[])? {
            let path = self.dir.join(&request.name);
            let bytes = bounded::read_bytes(
                &path,
                request.size.expect("archive request with a declared size"),
                Symlinks::Refuse,
            )
            .map_err(|e| e.to_string())?;
            let digest = index::sha256(&bytes);
            if digest != request.sha256 {
                return Err(format!(
                    "{}: its SHA-256 does not match the signed descriptor",
                    path.display()
                ));
            }
            put(&digest, &bytes)?;
            assets.push(Asset {
                name: request.name,
                sha256: digest,
            });
        }
        put(&version.sha256, source)?;
        let source_tag = version
            .sunrise
            .as_deref()
            .expect("source pin checked before preparing binaries");
        let fit_releases = self
            .releases
            .iter()
            .filter(|r| r.tag == source_tag || r.tag == release.tag)
            .cloned()
            .collect::<Vec<_>>();
        let mut version = registry::checked_version(
            &checked,
            &checked.versions[0],
            &Sources {
                files: scratch.path(),
                registry: scratch.path(),
                release: index::FILE_HOST,
                site: "https://example.invalid/",
                generated: String::new(),
                releases: Some(&fit_releases),
                nightly: None,
                binary_checks: Some(builds::Checks {
                    sdk_data: self.sdk_data,
                    source: sunrise_native_build::prebuilt::release_source,
                }),
            },
            &[],
        )?;
        fit::verify_submission(
            &entry.id,
            &mut version,
            &fit_releases,
            scratch.path(),
            self.sdk_data,
        )?;
        Ok(Submission {
            build,
            descriptor_signature_name,
            assets,
            verified_files: scratch,
        })
    }
}
