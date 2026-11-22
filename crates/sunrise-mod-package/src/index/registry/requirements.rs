//! Advisory check of the packaged Cargo requirements against the pinned game's crates.io versions.
use super::{Entry, Kind, Review, hex64, read_regular};
use crate::index::{MAX_FILE, sha256};
use crate::native_route::game_registry_pin_conflicts;
use semver::{Prerelease, Version, VersionReq};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Seek};
use std::path::{Component, Path, PathBuf};

const MAX_GAME_LOCK_BYTES: u64 = 16 << 20;

#[derive(Deserialize)]
struct Lock {
    package: Vec<Pin>,
}

#[derive(Deserialize)]
struct Pin {
    name: String,
    version: Version,
    source: Option<String>,
}

impl Review {
    /// Check only new or edited native version rows. Unreadable inputs don't turn this advisory
    /// into a refusal; the registry's package checks still own validation of the listed files.
    /// `crates_io_source` is the canonical Cargo source ID supplied by the registry tool.
    pub fn check_native_requirements(
        &mut self,
        base: &[Entry],
        head: &[Entry],
        files: &Path,
        sdk_data: Option<&Path>,
        crates_io_source: &str,
    ) {
        let mut locks = BTreeMap::new();
        for entry in head.iter().filter(|entry| entry.kind == Kind::NativeMod) {
            let old = base.iter().find(|old| old.id == entry.id);
            for row in entry.versions.iter().filter(|row| {
                old.is_none_or(|old| !old.versions.contains(row)) && hex64(&row.sha256).is_ok()
            }) {
                let Ok(bytes) = read_regular(files, Path::new(&row.sha256), MAX_FILE) else {
                    continue;
                };
                if sha256(&bytes) != row.sha256 {
                    continue;
                }
                let Ok(mut zip) = zip::ZipArchive::new(Cursor::new(bytes)) else {
                    continue;
                };
                let Ok(package) = crate::Package::read_zip(&mut zip) else {
                    continue;
                };
                let Some(native) = package.manifest.native else {
                    continue;
                };
                let build = native.sunrise;
                let pins = locks.entry(build.clone()).or_insert_with(|| {
                    let pins = sdk_data.and_then(|dir| game_pins(dir, &build, crates_io_source));
                    if pins.is_none() {
                        self.notes.push(format!(
                            "build {build}: native dependency requirements were not compared (game Cargo.lock unavailable)"
                        ));
                    }
                    pins
                });
                let Some(pins) = pins else { continue };
                for (manifest, package, requirement, game) in manifest_conflicts(&mut zip, pins) {
                    self.warnings.push(format!(
                        "{} {}: {manifest:?} requires {package} {requirement:?}; game build {} locks {game}. This mod cannot build with the game's version of {package}; loosen the requirement unless the other version is needed.",
                        entry.id, row.version, &build[..12],
                    ));
                }
            }
        }
    }
}

/// Read only the locked crates.io names and versions. The SDK supplies this file; the mod's own
/// lockfile has no bearing on whether its Cargo requirements admit the game's versions.
fn game_pins(
    sdk_data: &Path,
    build: &str,
    crates_io_source: &str,
) -> Option<BTreeMap<String, BTreeSet<Version>>> {
    let path = Path::new(build).join("Cargo.lock");
    let bytes = read_regular(sdk_data, &path, MAX_GAME_LOCK_BYTES).ok()?;
    let lock = toml::from_slice::<Lock>(&bytes).ok()?;
    let mut pins = BTreeMap::<_, BTreeSet<_>>::new();
    for pin in lock.package {
        if pin.source.as_deref() == Some(crates_io_source) {
            pins.entry(pin.name).or_default().insert(pin.version);
        }
    }
    Some(pins)
}

/// Walk only manifests reachable from the root package, including target and host dependencies.
fn manifest_conflicts<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    pins: &BTreeMap<String, BTreeSet<Version>>,
) -> BTreeSet<(String, String, String, String)> {
    const TABLES: [&str; 5] = [
        "dependencies",
        "dev-dependencies",
        "build-dependencies",
        "dev_dependencies",
        "build_dependencies",
    ];
    let mut todo = vec![PathBuf::new()];
    let mut seen = BTreeSet::new();
    let mut conflicts = BTreeSet::new();
    while let Some(dir) = todo.pop() {
        let name = dir.join("Cargo.toml").to_string_lossy().replace('\\', "/");
        if !crate::manifest::source_file(&name) || !seen.insert(name.clone()) {
            continue;
        }
        let Ok(file) = zip.by_name(&name) else {
            continue;
        };
        let size = file.size(); // Package::read_zip already bounded the source entries.
        let mut text = String::new();
        if file.take(size + 1).read_to_string(&mut text).is_err() || text.len() as u64 != size {
            continue;
        }
        let Ok(manifest) = text.parse::<toml::Table>() else {
            continue;
        };
        let targets = manifest.get("target").and_then(toml::Value::as_table);
        let sections = std::iter::once(&manifest).chain(
            targets
                .into_iter()
                .flat_map(|targets| targets.values().filter_map(toml::Value::as_table)),
        );
        let dependencies = sections
            .flat_map(|section| {
                TABLES
                    .iter()
                    .filter_map(|key| section.get(*key).and_then(toml::Value::as_table))
            })
            .flatten();
        for (key, dep) in dependencies {
            if let Some(path) = dep.get("path") {
                if let Some(inside) = path.as_str().and_then(|path| local_path(&dir, path)) {
                    todo.push(inside);
                }
                continue;
            }
            if ["git", "registry-index", "workspace"]
                .iter()
                .any(|key| dep.get(key).is_some())
                || dep
                    .get("registry")
                    .is_some_and(|registry| registry.as_str() != Some("crates-io"))
            {
                continue;
            }
            let package = dep
                .get("package")
                .and_then(toml::Value::as_str)
                .unwrap_or(key);
            let Some(game) = pins.get(package) else {
                continue;
            };
            let Some(raw) = dep
                .as_str()
                .or_else(|| dep.get("version").and_then(toml::Value::as_str))
            else {
                continue;
            };
            let Ok(requirement) = VersionReq::parse(raw) else {
                continue;
            };
            if game.iter().any(|pin| requirement.matches(pin)) {
                continue;
            }
            let compatible: Vec<_> = game
                .iter()
                .filter(|pin| accepts_another_on_line(&requirement, pin))
                .map(Version::to_string)
                .collect();
            if !compatible.is_empty() {
                conflicts.insert((
                    name.clone(),
                    package.to_owned(),
                    raw.to_owned(),
                    compatible.join(", "),
                ));
            }
        }
    }
    conflicts
}

/// Resolve a dependency relative to its archive manifest, never consulting the host filesystem.
fn local_path(dir: &Path, path: &str) -> Option<PathBuf> {
    if path.contains(['\\', ':']) {
        return None;
    }
    let mut inside = dir.to_owned();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => inside.push(part),
            Component::CurDir => {}
            Component::ParentDir if inside.pop() => {}
            _ => return None,
        }
    }
    Some(inside)
}

/// Without the registry index, check whether the requirement accepts a version that a build
/// would have to put in place of the game's pin.
fn accepts_another_on_line(requirement: &VersionReq, pin: &Version) -> bool {
    let matches = |version: Version| {
        game_registry_pin_conflicts(pin, &version) && requirement.matches(&version)
    };
    // The first matching stable version is the lowest of the pin's major or minor, the release
    // of a prerelease pin, or on or immediately after a comparison boundary. Partial comparators cover a whole major or minor;
    // advance the last specified component.
    matches(Version::new(pin.major, 0, 0))
        || matches(Version::new(pin.major, pin.minor, 0))
        || matches(Version::new(pin.major, pin.minor, pin.patch))
        || requirement.comparators.iter().any(|cmp| {
            let (major, minor, patch) = (cmp.major, cmp.minor.unwrap_or(0), cmp.patch.unwrap_or(0));
            let next = cmp
                .patch
                .and_then(|patch| patch.checked_add(1))
                .map(|patch| Version::new(major, minor, patch))
                .or_else(|| {
                    cmp.minor
                        .and_then(|minor| minor.checked_add(1))
                        .map(|minor| Version::new(major, minor, 0))
                })
                .or_else(|| major.checked_add(1).map(|major| Version::new(major, 0, 0)));
            matches(Version::new(major, minor, patch))
                || next.is_some_and(matches)
                // A prerelease can only match on a triple with an explicit prerelease comparator.
                || (!cmp.pre.is_empty()
                    && ["0".into(), cmp.pre.to_string(), format!("{}.0", cmp.pre)]
                        .iter()
                        .any(|pre| {
                            Prerelease::new(pre).is_ok_and(|pre| {
                                let mut version = Version::new(major, minor, patch);
                                version.pre = pre;
                                matches(version)
                            })
                        }))
        })
}
