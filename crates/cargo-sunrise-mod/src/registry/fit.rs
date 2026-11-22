//! Publish fit from workflow-supplied SDK files. This route never fetches a build or runs a mod.
use crate::sdk_cmds::{PackageSubject, Subject, compare_fit, compare_game, package_subject};
use crate::sdk_fetch::{BUILD_DATA, SURFACE};
use mod_surface::{
    ReadError, Surface,
    uses::{Change, Fit},
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use sunrise_mod_package::{
    build_data,
    index::{BuildVerdict, FitVerdict, Index, Kind, SunriseRelease, UnjudgedBuild, Version},
    sdk,
};
use sunrise_native_build::game_uses;

struct Build {
    sdk: sdk::Index,
    surface: Result<Surface, String>,
    surface_bytes: Vec<u8>,
}

impl Build {
    fn read(directory: &Path, commit: &str) -> Result<Self, String> {
        let read = |name| {
            let path = directory.join(name);
            std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
        };
        let bytes = read(sdk::INDEX)?;
        let sdk = sdk::Index::read_for_fit(&bytes, commit).map_err(|e| e.to_string())?;
        let surface_bytes = read(SURFACE)?;
        sdk.verify(SURFACE, surface_bytes.as_slice())
            .map_err(|e| e.to_string())?;
        let surface = match Surface::read(&surface_bytes) {
            Err(e @ ReadError::Schema(_)) => Err(format!("{SURFACE}: {e}")),
            Err(e) => return Err(e.to_string()),
            Ok(surface) => Ok(surface),
        };
        Ok(Self {
            sdk,
            surface,
            surface_bytes,
        })
    }
}

/// Group release tags by commit: the Nightly and a stable release can share a build.
pub(super) fn publish(
    index: &mut Index,
    files: &Path,
    sdk_data: Option<&Path>,
) -> Result<(), String> {
    let releases = index.releases.as_deref().unwrap_or_default();
    if releases.is_empty() && index.nightly_build.is_none() {
        return Ok(());
    }
    let versions = index.projects.iter_mut().flat_map(|project| {
        project.versions.iter_mut().filter_map(|version| {
            let is_native = match project.kind {
                Kind::NativeMod => true,
                Kind::Mod => false,
                _ => return None,
            };
            Some((project.id.as_str(), version, is_native))
        })
    });
    judge(
        versions,
        releases,
        index
            .nightly_build
            .as_ref()
            .map(|build| build.commit.as_str()),
        &mut index.unjudged,
        files,
        sdk_data,
    )
}

fn judge<'a>(
    versions: impl Iterator<Item = (&'a str, &'a mut Version, bool)>,
    releases: &[SunriseRelease],
    nightly: Option<&str>,
    unjudged: &mut Vec<UnjudgedBuild>,
    files: &Path,
    sdk_data: Option<&Path>,
) -> Result<(), String> {
    let mut pinned_versions = Vec::new();
    let mut lines = BTreeSet::new();
    for (id, version, is_native) in versions {
        let Some(tag) = &version.sunrise else {
            continue;
        };
        let made_for = releases.iter().find(|r| r.tag == *tag).ok_or_else(|| {
            format!(
                "{id} {}: use --releases with Sunrise release {tag:?} to compute fit",
                version.version
            )
        })?;
        lines.insert(made_for.line.as_str());
        pinned_versions.push((version, made_for, is_native));
    }
    let mut targets = BTreeMap::<&str, Vec<(&str, Option<&str>)>>::new();
    for release in releases.iter().filter(|r| lines.contains(r.line.as_str())) {
        targets
            .entry(&release.commit)
            .or_default()
            .push((&release.tag, Some(&release.line)));
    }
    if let Some(nightly) = nightly
        && !pinned_versions.is_empty()
    {
        targets.entry(nightly).or_default().push(("nightly", None));
    }
    let Some((&first_commit, first_keys)) = targets.first_key_value() else {
        return Ok(());
    };
    let sdk_data = sdk_data.ok_or_else(|| {
        format!(
            "Sunrise {} ({first_commit}): give --sdk-data <dir> with the build's SDK files",
            first_keys[0].0
        )
    })?;
    // Older uses lists can read hook versions from their pinned surface, even if that surface
    // cannot be read as a verdict target. Keep those bytes, not every build's game-code tables.
    let mut builds = BTreeMap::new();
    for (&commit, keys) in &targets {
        let build = Build::read(&sdk_data.join(commit), commit)
            .map_err(|error| format!("Sunrise {} ({commit}): {error}", keys[0].0))?;
        builds.insert(commit, build);
    }

    let mut packages = HashMap::new();
    for (commit, keys) in targets {
        let build = &builds[commit];
        let data = match game_uses::Data::read(&sdk_data.join(commit).join(BUILD_DATA), commit) {
            Err(e @ build_data::ReadError::Schema { .. }) => Err(e.to_string()),
            Err(e) => return Err(format!("Sunrise {} ({commit}): {e}", keys[0].0)),
            Ok(data) => Ok(data),
        };
        let inputs = build
            .surface
            .as_ref()
            .map_err(String::as_str)
            .and_then(|surface| {
                data.as_ref()
                    .map_err(String::as_str)
                    .map(|data| (surface, data))
            });
        let (surface, data) = match inputs {
            Ok(inputs) => inputs,
            Err(reason) => {
                let names: Vec<_> = keys.iter().map(|&(name, _)| name).collect();
                eprintln!("warning: Sunrise {} ({commit}): {reason}", names.join(", "));
                unjudged.extend(names.into_iter().map(|name| UnjudgedBuild {
                    build: name.to_owned(),
                    reason: reason.to_owned(),
                }));
                continue;
            }
        };
        for (version, made_for, is_native) in &mut pinned_versions {
            let mut applicable = keys
                .iter()
                .filter(|(_, line)| line.is_none_or(|line| line == made_for.line))
                .peekable();
            if applicable.peek().is_none() {
                continue;
            }
            let pinned = commit == made_for.commit;
            let fit = if version.game != build.sdk.build.game {
                breaks("minecraft")
            } else if *is_native {
                // Assembly has already verified the package. An unreadable uses list is not
                // evidence of fit, just as a package without a list is not.
                let (subject, baseline) = packages
                    .entry(version.file.sha256.clone())
                    .or_insert_with(|| {
                        (
                            package_subject(&files.join(&version.file.sha256)).ok(),
                            None,
                        )
                    });
                let known = |pin: &str| {
                    builds
                        .get(pin)
                        .map(|b| b.surface_bytes.as_slice())
                        .ok_or_else(|| format!("no SDK data for {pin}"))
                };
                native(subject, baseline, &build.sdk, surface, data, pinned, known)
            } else if version.api != build.sdk.content_api {
                breaks("content-api")
            } else if pinned {
                FitVerdict::MadeFor
            } else {
                FitVerdict::Fits
            };
            let mut targets: Vec<_> = version
                .native_binaries
                .iter()
                .filter(|b| b.build == commit)
                .map(|b| b.target.clone())
                .collect();
            targets.sort();
            let row = BuildVerdict { fit, targets };
            version
                .verdicts
                .extend(applicable.map(|(key, _)| ((*key).to_owned(), row.clone())));
        }
    }
    Ok(())
}

/// Additional check and review work only for entries that submit stable binaries.
pub(super) fn verify_binaries(
    index: &mut Index,
    files: &Path,
    sdk_data: Option<&Path>,
) -> Result<(), String> {
    if index
        .projects
        .iter()
        .flat_map(|p| &p.versions)
        .any(|v| !v.native_binaries.is_empty())
    {
        publish(index, files, sdk_data)?;
        check_binaries(index)?;
    }
    Ok(())
}

/// Judge a single author's proposed native build, without requiring its project page or
/// SDK data for any release other than the source pin and the candidate target.
pub(crate) fn verify_submission(
    id: &str,
    version: &mut Version,
    releases: &[SunriseRelease],
    files: &Path,
    sdk_data: &Path,
) -> Result<(), String> {
    judge(
        std::iter::once((id, &mut *version, true)),
        releases,
        None,
        &mut Vec::new(),
        files,
        Some(sdk_data),
    )
    .map_err(|e| {
        format!("{e}; give --sdk-data <dir> with the SDKs for the source pin and the target build")
    })?;
    let mut errors = Vec::new();
    binary_errors(id, version, releases, &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(super::lines(errors))
    }
}

/// A verified binary supplies the exact build for an `exact` verdict. It cannot turn a
/// known break (or missing SDK evidence) into a fit.
pub(super) fn check_binaries(index: &Index) -> Result<(), String> {
    let releases = index.releases.as_deref().unwrap_or_default();
    let mut errors = Vec::new();
    for project in &index.projects {
        for version in &project.versions {
            binary_errors(&project.id, version, releases, &mut errors);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(super::lines(errors))
    }
}

fn binary_errors(
    id: &str,
    version: &Version,
    releases: &[SunriseRelease],
    errors: &mut Vec<String>,
) {
    let commits: BTreeSet<_> = version
        .native_binaries
        .iter()
        .map(|b| b.build.as_str())
        .collect();
    for commit in commits {
        let verdict = releases
            .iter()
            .filter(|r| r.commit == commit)
            .find_map(|r| version.verdicts.get(&r.tag));
        let reason = match verdict.map(|v| &v.fit) {
            Some(FitVerdict::Breaks { changed }) => {
                format!("source does not fit: {}", changed.join(", "))
            }
            Some(FitVerdict::Rebuild { review }) => {
                format!(
                    "source does not fit: function bodies changed: {}",
                    review.join(", ")
                )
            }
            None => "no stable fit verdict for this build".to_owned(),
            Some(_) => continue,
        };
        errors.push(format!(
            "{id} {}: native build {commit}: {reason}",
            version.version
        ));
    }
}

fn native<'a>(
    subject: &mut Option<PackageSubject>,
    baseline: &mut Option<Result<(), String>>,
    sdk: &sdk::Index,
    surface: &Surface,
    data: &game_uses::Data,
    pinned: bool,
    baseline_surface: impl FnOnce(&str) -> Result<&'a [u8], String>,
) -> FitVerdict {
    let Some(subject) = subject else {
        return exact("package manifest could not be read");
    };
    if subject.content_api != sdk.content_api {
        return breaks("content-api");
    }
    if pinned {
        return FitVerdict::MadeFor;
    }
    let (manifest, table) = match &mut subject.uses {
        Ok(Subject::Table {
            manifest, table, ..
        }) => (manifest, table),
        Ok(Subject::NotApplicable { reason, .. }) => return exact(reason),
        Err(_) => return exact("no readable uses list"),
    };
    let game = compare_game(manifest, table, &sdk.mixins.rev, || {
        Ok(data.entries(manifest))
    });
    let comparison = compare_fit(table, surface, game, baseline, baseline_surface);
    let game = comparison.compared;
    let (mut changed, exact_reason) = match comparison.report.verdict {
        Fit::DoesNotFit(items) => (items, None),
        Fit::Unversioned(_) => (
            Vec::new(),
            Some("the uses list does not record hook versions"),
        ),
        Fit::Fits => (Vec::new(), None),
    };
    changed.extend(game.misfits);
    if !changed.is_empty() {
        if exact_reason.is_none()
            && comparison.report.deferred.is_none()
            && !game.unrecorded
            && game.unjudged.is_empty()
            && let Some(review) = changed
                .iter()
                .map(|m| match &m.change {
                    Change::BodyChanged { function } => Some(function.clone()),
                    _ => None,
                })
                .collect::<Option<BTreeSet<_>>>()
        {
            return FitVerdict::Rebuild {
                review: review.into_iter().collect(),
            };
        }
        return FitVerdict::Breaks {
            changed: changed
                .into_iter()
                .map(|m| match m.change {
                    Change::BodyChanged { .. } | Change::DeclarationChanged { .. } => m.to_string(),
                    _ => m.id,
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .take(5)
                .collect(),
        };
    }
    if let Some(reason) = comparison.report.deferred {
        return FitVerdict::Exact { reason };
    }
    if let Some(why) = exact_reason {
        return exact(why);
    }
    if game.unrecorded || !game.unjudged.is_empty() {
        return exact("game code could not be fully compared");
    }
    FitVerdict::Fits
}

fn breaks(item: &str) -> FitVerdict {
    FitVerdict::Breaks {
        changed: vec![item.to_owned()],
    }
}

fn exact(reason: &str) -> FitVerdict {
    FitVerdict::Exact {
        reason: reason.to_owned(),
    }
}
