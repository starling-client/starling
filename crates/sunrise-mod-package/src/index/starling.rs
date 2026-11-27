//! Starling's part of index.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Index {
    /// Starling: `other`'s projects and takedowns added to this index's, for a catalog that
    /// lists two registries (Sunrise's and Horizon's). A project both name is this index's.
    pub fn merge(&mut self, other: Index) {
        for project in other.projects {
            if self.project(&project.id).is_none() {
                self.projects.push(project);
            }
        }
        self.revoked.extend(other.revoked);
        self.left_out.extend(other.left_out);
    }
}

/// Starling: where Sunrise's registry publishes its signed v2 index (sunrise-addresses
/// `MOD_REGISTRY_INDEX`). Sunrise stopped serving the old `index.json` on 2026-11-26.
pub const V2_INDEX_URL: &str = "https://sunrise-org.github.io/sunrise-mods/v2/index.signed";
/// The registry's public key, which signs [`V2_INDEX_URL`] (cargo-sunrise-mod's registry_source.rs).
pub const V2_KEY: &str = "RWTNJaFJ4AekHG/RgVrgmJpvy9B2EaeqbdSNuseKUx2XPxaOZkChLV0q";
/// The v2 index's image paths are relative to the folder of its `index.json`.
const V2_PAGES: &str = "https://sunrise-org.github.io/sunrise-mods/v2/";
/// The `api` of a version read from the v2 index. v2 lists no API line, side or capabilities (the
/// package says them), so such a version is checked by [`check_v2_file`], not [`check_file`].
pub const V2_API: &str = "registry-v2";

/// Starling: Sunrise's signed v2 index (`v2/index.signed`, sunrise-mods docs/registry-v2.md
/// "Reader rules") as the index the launcher's browser reads, and its serial. The first four lines
/// are the minisign signature over every byte after them, checked with `key` before anything is
/// parsed; only schema 2 is read. Each project is listed under its in-game ID (`mod`), the slot
/// a package fills: when two projects fill one slot, the first listed is kept and the other is
/// [`Index::left_out`]. Every version is for `game`, the registry's only one.
pub fn read_v2(signed: &[u8], key: &str, game: &str) -> Result<(Index, u64), String> {
    let at = signed
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'\n')
        .nth(3)
        .map(|(at, _)| at + 1)
        .ok_or("fewer than the four signature lines before the index")?;
    let (signature, bytes) = signed.split_at(at);
    let signature = std::str::from_utf8(signature)
        .map_err(|_| "the signature lines are not UTF-8".to_string())?;
    let key =
        minisign_verify::PublicKey::from_base64(key.trim()).map_err(|e| format!("key: {e}"))?;
    let signature =
        minisign_verify::Signature::decode(signature).map_err(|e| format!("signature: {e}"))?;
    key.verify(bytes, &signature, false)
        .map_err(|_| "the signature doesn't match the registry's key".to_string())?;
    let json: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if json["schema"].as_u64() != Some(2) {
        return Err(format!("schema {} where 2 is expected", json["schema"]));
    }
    let serial = json["serial"].as_u64().ok_or("the index has no serial")?;
    Ok((from_v2(&json, game), serial))
}

/// The signed index a launcher keeps in its folder, and the highest serial it accepted.
const V2_KEPT: &str = "sunrise-index.signed";
const V2_SERIAL: &str = "sunrise-index.serial";

/// Starling: the v2 index for a launcher's browser, from `fetched` (the bytes at [`V2_INDEX_URL`],
/// or why they couldn't be had), kept in `folder`. A fetched index whose signature holds and
/// whose serial is not lower than the last one accepted there is kept (the reader rules: an older
/// serial is a rollback) and read; otherwise the copy kept last time is read, checked again. The
/// error is the fetch's when neither can be read.
pub fn v2_index(
    fetched: Result<Vec<u8>, String>,
    folder: &Path,
    game: &str,
) -> Result<Index, String> {
    let kept = folder.join(V2_KEPT);
    let floor = std::fs::read_to_string(folder.join(V2_SERIAL))
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok());
    let live = fetched.and_then(|bytes| {
        let (index, serial) = read_v2(&bytes, V2_KEY, game)?;
        if floor.is_some_and(|floor| serial < floor) {
            return Err(format!(
                "the registry served serial {serial}, older than {} accepted before",
                floor.unwrap_or_default()
            ));
        }
        // Each written whole by a rename, so a stopped launcher never leaves half a file.
        let write = |name: &str, bytes: &[u8]| {
            let part = folder.join(format!("{name}.part"));
            std::fs::create_dir_all(folder)
                .and_then(|()| std::fs::write(&part, bytes))
                .and_then(|()| std::fs::rename(&part, folder.join(name)))
        };
        let _ = write(V2_KEPT, &bytes);
        if floor.is_none_or(|floor| serial > floor) {
            let _ = write(V2_SERIAL, serial.to_string().as_bytes());
        }
        Ok(index)
    });
    live.or_else(|e| {
        let bytes = std::fs::read(&kept).map_err(|_| e.clone())?;
        read_v2(&bytes, V2_KEY, game)
            .map(|(index, _)| index)
            .map_err(|_| e)
    })
}

/// [`read_v2`]'s index from the v2 JSON, once its signature is checked. A project, release or
/// takedown outside the shapes the registry document gives is left out with the reason.
pub fn from_v2(json: &serde_json::Value, game: &str) -> Index {
    use serde_json::Value;
    let text = |v: &Value, key: &str| v[key].as_str().unwrap_or_default().to_owned();
    let texts = |v: &Value, key: &str| -> Vec<String> {
        v[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut index = Index {
        schema: SCHEMA,
        generated: text(json, "generated"),
        ..Index::default()
    };
    let mut slots = BTreeMap::new();
    let empty = Vec::new();
    for p in json["projects"].as_array().unwrap_or(&empty) {
        let (project, slot) = (text(p, "id"), text(p, "mod"));
        let left_out = |reason: String| LeftOut {
            id: if slot.is_empty() {
                project.clone()
            } else {
                slot.clone()
            },
            version: None,
            reason,
        };
        if !v2_project_id(&project) || !valid_id(&slot) {
            index.left_out.push(left_out(format!(
                "project {project:?} or its slot isn't well formed"
            )));
            continue;
        }
        let Ok(kind) = serde_json::from_value::<Kind>(p["kind"].clone()) else {
            index
                .left_out
                .push(left_out(format!("project {project}: no kind")));
            continue;
        };
        if index.project(&slot).is_some() {
            index.left_out.push(left_out(format!(
                "project {project} fills the slot {slot} another project listed first fills"
            )));
            continue;
        }
        let mut versions = Vec::new();
        for r in p["releases"].as_array().unwrap_or(&empty) {
            let file = &r["file"];
            let (url, sha256) = (text(file, "url"), text(file, "sha256"));
            let version = text(r, "version");
            let size = file["size"].as_u64().unwrap_or(0);
            if semver::Version::parse(&version).is_err()
                || !v2_digest(&sha256)
                || !url.starts_with("https://")
                || size == 0
            {
                index.left_out.push(LeftOut {
                    id: slot.clone(),
                    version: Some(version),
                    reason: "a release whose version, file or digest isn't well formed".into(),
                });
                continue;
            }
            let name = url.rsplit('/').next().unwrap_or_default().to_owned();
            versions.push(Version {
                version,
                published: text(r, "published"),
                changelog: String::new(),
                api: V2_API.into(),
                side: None,
                capabilities: Capabilities::default(),
                game: game.into(),
                sunrise: None,
                verdicts: BTreeMap::new(),
                dependencies: None,
                requires: Vec::new(),
                native_binaries: Vec::new(),
                native_binary_refusals: Vec::new(),
                file: File {
                    name,
                    url,
                    size,
                    sha256,
                    signature: None,
                },
            });
        }
        if versions.is_empty() {
            index
                .left_out
                .push(left_out(format!("project {project}: no release to list")));
            continue;
        }
        versions.sort_by(|a, b| {
            let (a, b) = (
                semver::Version::parse(&a.version),
                semver::Version::parse(&b.version),
            );
            b.ok().cmp(&a.ok())
        });
        let image = |path: &str| v2_image(path).then(|| format!("{V2_PAGES}{path}"));
        let repository = text(&p["repository"], "name");
        let name = Some(text(p, "name"))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| slot.clone());
        index.projects.push(Project {
            id: slot.clone(),
            kind,
            name,
            summary: text(p, "summary"),
            description: text(p, "description"),
            authors: texts(p, "authors"),
            owners: texts(p, "owners"),
            license: text(p, "license"),
            source: [text(p, "source"), text(p, "homepage")]
                .into_iter()
                .find(|s| s.starts_with("https://"))
                .unwrap_or_else(|| format!("https://github.com/{repository}")),
            icon: p["icon"].as_str().and_then(image),
            categories: texts(p, "categories"),
            tags: Vec::new(),
            screenshots: p["screenshots"]
                .as_array()
                .unwrap_or(&empty)
                .iter()
                .filter_map(|s| s["path"].as_str().and_then(image))
                .collect(),
            key: None,
            versions,
            nightly: None,
            downloads: 0,
            stars: 0,
        });
        slots.insert(project, slot);
    }
    for r in json["revoked"].as_array().unwrap_or(&empty) {
        let digest = r["sha256"].as_str().map(str::to_owned);
        match slots.get(r["project"].as_str().unwrap_or_default()) {
            Some(slot) if digest.as_deref().is_none_or(v2_digest) => index.revoked.push(Revoked {
                id: slot.clone(),
                sha256: digest,
                reason: "taken down by Sunrise's registry".into(),
                date: String::new(),
            }),
            _ => {}
        }
    }
    index
}

/// Starling: the check a listed version's downloaded file takes: [`check_v2_file`] for one read
/// from the v2 index, else [`check_file`].
pub fn check_listed_file(
    index: &Index,
    project: &Project,
    version: &Version,
    path: &Path,
) -> Result<Trust, String> {
    match version.api == V2_API {
        true => check_v2_file(index, project, version, path),
        false => check_file(index, project, version, path),
    }
}

/// Starling: [`check_file`] for a version read from the v2 index ([`V2_API`]): its size and
/// SHA-256 are the ones the registry signed, it wasn't taken down, and the package's own manifest
/// names this mod and version. v2 lists no API line, side or capabilities to compare them with:
/// the game reads them from the package as it loads it.
pub fn check_v2_file(
    index: &Index,
    project: &Project,
    version: &Version,
    path: &Path,
) -> Result<Trust, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if size != version.file.size {
        return Err(format!(
            "{} bytes, the index lists {}",
            size, version.file.size
        ));
    }
    let hash = sha256_file(path)?;
    if hash != version.file.sha256 {
        return Err("its SHA-256 isn't the one the index lists: nothing was installed".into());
    }
    if let Some(r) = index.revoked(&project.id, &hash) {
        return Err(format!("{} was taken down: {}", project.name, r.reason));
    }
    if matches!(project.kind, Kind::Other(_)) {
        return Err(format!(
            "this Sunrise can't install a {} yet",
            project.kind.name()
        ));
    }
    let facts = inspect(&project.kind, path)?;
    if let Some(id) = facts.id.as_ref().filter(|id| **id != project.id) {
        return Err(format!("it is {id}, not {}", project.id));
    }
    if let Some(found) = facts
        .version
        .as_ref()
        .filter(|found| !same_version(&project.kind, found, &version.version))
    {
        return Err(format!(
            "it says version {found}, the index lists {}",
            version.version
        ));
    }
    // The registry's signature over the index pins this digest.
    Ok(Trust::Verified)
}

/// A SHA-256 as the v2 index writes it: 64 lowercase hex digits.
fn v2_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The registry's project IDs: eight base62 characters.
fn v2_project_id(id: &str) -> bool {
    id.len() == 8 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// An image path as the v2 index gives them, `pages/<id>/<sha256>.png|jpg`: nothing else becomes
/// an address.
fn v2_image(path: &str) -> bool {
    let mut parts = path.split('/');
    let (Some("pages"), Some(owner), Some(file), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let digest = file
        .strip_suffix(".png")
        .or_else(|| file.strip_suffix(".jpg"));
    !owner.is_empty()
        && owner.bytes().all(|b| b.is_ascii_alphanumeric())
        && digest.is_some_and(v2_digest)
}

#[cfg(test)]
mod v2_tests {
    use super::*;

    fn index() -> serde_json::Value {
        let release = |version: &str, digest: char| {
            serde_json::json!({"version": version, "published": "2026-11-26", "fits": [],
                "file": {"url": format!("https://example.com/{version}.smod"), "size": 10,
                    "sha256": digest.to_string().repeat(64), "attestation": "a"}})
        };
        serde_json::json!({
            "schema": 2, "serial": 9, "generated": "2026-11-27T00:00:00Z",
            "projects": [
                {"id": "AbCdEfGh", "slug": "villagefolk", "mod": "villagefolk", "kind": "native-mod",
                 "name": "Villagefolk", "summary": "Names", "authors": ["EAnathos"], "license": null,
                 "repository": {"id": 1, "name": "EAnathos/sunrise-villagefolk"},
                 "icon": format!("pages/AbCdEfGh/{}.png", "c".repeat(64)),
                 "screenshots": [{"path": "../../etc/passwd"}, {"path": format!("pages/AbCdEfGh/{}.jpg", "d".repeat(64))}],
                 "releases": [release("1.0.0", 'a'), release("1.10.0", 'b'), release("1.2.0", 'e')]},
                {"id": "ZyXwVuTs", "mod": "villagefolk", "kind": "native-mod", "name": "Another",
                 "releases": [release("2.0.0", 'f')]},
                {"id": "Short", "mod": "short", "kind": "mod", "releases": [release("1.0.0", 'a')]},
            ],
            "revoked": [{"project": "AbCdEfGh", "sha256": "a".repeat(64)}, {"project": "Unknown1", "sha256": null}],
        })
    }

    #[test]
    fn projects_by_slot_with_versions_newest_first_and_page_urls() {
        let index = from_v2(&index(), "1.21.10");
        assert_eq!(index.projects.len(), 1);
        let p = &index.projects[0];
        assert_eq!(
            (p.id.as_str(), p.name.as_str(), p.kind.clone()),
            ("villagefolk", "Villagefolk", Kind::NativeMod)
        );
        assert_eq!(p.source, "https://github.com/EAnathos/sunrise-villagefolk");
        assert_eq!(p.license, "");
        let versions: Vec<&str> = p.versions.iter().map(|v| v.version.as_str()).collect();
        assert_eq!(versions, ["1.10.0", "1.2.0", "1.0.0"]);
        assert!(
            p.versions
                .iter()
                .all(|v| v.api == V2_API && v.game == "1.21.10")
        );
        assert_eq!(p.versions[0].file.name, "1.10.0.smod");
        assert_eq!(
            p.icon.as_deref(),
            Some(
                format!(
                    "https://sunrise-org.github.io/sunrise-mods/v2/pages/AbCdEfGh/{}.png",
                    "c".repeat(64)
                )
                .as_str()
            )
        );
        assert_eq!(
            p.screenshots.len(),
            1,
            "a path outside pages/ never becomes an address"
        );
        let left: Vec<&str> = index.left_out.iter().map(|l| l.reason.as_str()).collect();
        assert!(
            left.iter()
                .any(|r| r.contains("ZyXwVuTs fills the slot villagefolk")),
            "{left:?}"
        );
        assert!(left.iter().any(|r| r.contains("\"Short\"")), "{left:?}");
    }

    #[test]
    fn takedowns_name_the_slot() {
        let index = from_v2(&index(), "1.21.10");
        assert_eq!(index.revoked.len(), 1);
        assert!(index.revoked("villagefolk", &"a".repeat(64)).is_some());
        assert!(index.revoked("villagefolk", &"b".repeat(64)).is_none());
    }

    #[test]
    fn an_index_without_its_signature_lines_or_with_another_key_is_refused() {
        let body = serde_json::to_vec(&index()).unwrap();
        assert!(read_v2(&body, V2_KEY, "1.21.10").is_err());
        let mut signed = b"untrusted comment: x\nRWQ\ntrusted comment: y\nAAAA\n".to_vec();
        signed.extend_from_slice(&body);
        assert!(read_v2(&signed, V2_KEY, "1.21.10").is_err());
    }
}
