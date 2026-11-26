//! The signed v2 registry index (`v2/index.signed`, sunrise-mods `docs/registry-v2.md` "Published
//! index"): only what the mod tool reads to take a library. Under schema 2 a reader ignores
//! fields it doesn't know, so the published page, build and fit fields are left out here.
use super::{Kind, MAX_FILE};
use serde::{Deserialize, Deserializer};

pub mod collection;

/// The only schema served at the v2 address; a breaking change raises it and moves the index.
pub const SCHEMA: u64 = 2;

#[derive(Debug, Deserialize)]
pub struct Index {
    pub schema: u64,
    /// Rises with every publish: a reader refuses one lower than it last accepted.
    pub serial: u64,
    /// Projects with at least one release.
    pub projects: Vec<Project>,
    pub revoked: Vec<Revoked>,
}

#[derive(Debug, Deserialize)]
pub struct Project {
    /// The permanent project ID, not the in-game ID.
    pub id: String,
    /// The in-game ID: the dependency slot the project provides.
    #[serde(rename = "mod")]
    pub mod_id: String,
    pub kind: Kind,
    pub repository: Repository,
    pub releases: Vec<Release>,
}

#[derive(Debug, Deserialize)]
pub struct Repository {
    /// GitHub's numeric repository ID, which a release's attestation must name.
    pub id: u64,
    /// `owner/name` when published; a rename changes it, never the ID.
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct Release {
    #[serde(deserialize_with = "semver_text")]
    pub version: semver::Version,
    /// The exact signer workflow ref the release's attestation records.
    pub workflow: String,
    pub file: File,
}

#[derive(Debug, Deserialize)]
pub struct File {
    pub url: String,
    pub size: u64,
    pub sha256: String,
    /// The `.sigstore.json` bundle attesting `sha256`.
    pub attestation: String,
}

#[derive(Debug, Deserialize)]
pub struct Revoked {
    pub project: String,
    /// One file's digest, or none for every file of the project.
    pub sha256: Option<String>,
}

impl Index {
    /// Read `index.signed`: its first four lines are the minisign signature file, and every byte
    /// after the fourth line feed is `index.json`, checked as served before it is parsed. One file,
    /// so the signature and the index always come from the same publish.
    pub fn verify(signed: &[u8], key: &str) -> Result<Self, String> {
        let split = signed
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .nth(3)
            .map(|(at, _)| at + 1)
            .ok_or("fewer than the four signature lines before the index")?;
        let (signature, bytes) = signed.split_at(split);
        let signature = std::str::from_utf8(signature)
            .map_err(|_| "the signature lines are not UTF-8".to_string())?;
        super::verify_signature_bytes(key, signature, bytes)
            .map_err(|_| "the signature doesn't match the registry's key".to_string())?;
        Self::parse(bytes)
    }

    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let index: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if index.schema != SCHEMA {
            return Err(format!(
                "schema {} where {SCHEMA} is expected",
                index.schema
            ));
        }
        // Readers name cache folders by project ID and digest: refuse the whole index rather than
        // let a field outside these shapes become a path.
        for project in &index.projects {
            if !project_id(&project.id) {
                return Err(format!(
                    "project ID {:?} is not 8 base62 characters",
                    project.id
                ));
            }
            for release in &project.releases {
                let file = &release.file;
                if !sha256(&file.sha256) {
                    return Err(format!(
                        "{} {}: SHA-256 {:?} is not 64 lowercase hexadecimal characters",
                        project.id, release.version, file.sha256
                    ));
                }
                if file.size == 0 || file.size > MAX_FILE {
                    return Err(format!(
                        "{} {}: size {} is outside 1 byte to {MAX_FILE} bytes",
                        project.id, release.version, file.size
                    ));
                }
            }
        }
        for row in &index.revoked {
            if !project_id(&row.project) || row.sha256.as_deref().is_some_and(|d| !sha256(d)) {
                return Err(format!(
                    "takedown of {:?} names an invalid project ID or SHA-256",
                    row.project
                ));
            }
        }
        Ok(index)
    }

    /// The takedown covering `project`'s file with this digest, or every file of it.
    pub fn revoked(&self, project: &str, sha256: &str) -> Option<&Revoked> {
        self.revoked.iter().find(|r| {
            r.project == project && r.sha256.as_deref().is_none_or(|digest| digest == sha256)
        })
    }
}

/// The registry's project IDs: eight case-sensitive base62 characters (sunrise-mods
/// `docs/registry-v2.md`, after Modrinth's `apps/labrinth/src/database/models/ids.rs`).
fn project_id(id: &str) -> bool {
    id.len() == 8 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn sha256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn semver_text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<semver::Version, D::Error> {
    let text = String::deserialize(deserializer)?;
    semver::Version::parse(&text)
        .map_err(|e| serde::de::Error::custom(format!("release version {text:?}: {e}")))
}

#[cfg(test)]
mod tests;
