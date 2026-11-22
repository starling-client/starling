//! `cargo sunrise-mod registry`: the registry repository's CI (`docs/modding/src/reference/registry-index.md`).
//! `releases` discovers stable Sunrise releases. Other commands work on local files; the workflow fetches
//! their inputs and publishes what they write.
use crate::options::Options;
use serde::de::DeserializeOwned;
use std::path::Path;

pub(crate) mod fit;
#[cfg(test)]
mod tests;

mod releases;
use sunrise_mod_package::index::{
    registry::{self, Sources},
    resolve::uninstallable_versions,
};

/// Where this registry publishes its release assets and its site (icons and screenshots).
const RELEASE: &str = "https://github.com/sunrise-org/sunrise-mods/releases/download/";
const SITE: &str = "https://sunrise-org.github.io/sunrise-mods/";

const USAGE: &str = "usage: cargo sunrise-mod registry build <checkout> --files <dir> --out <index.json> [--release <url>] [--site <url>] [--generated <time>] [--counts <counts.json>] [--releases <file>] [--nightly <file>] [--nightly-build <file>] [--sdk-data <dir>]
       cargo sunrise-mod registry review <base checkout> <head checkout> --author <login> [--files <dir>] [--releases <file>] [--sdk-data <dir>] [--json]
       cargo sunrise-mod registry check <base checkout> <head checkout> --files <dir> [--releases <file>] [--sdk-data <dir>] [--json]
       cargo sunrise-mod registry list <checkout> [--build-assets [--files <dir>]]
       cargo sunrise-mod registry release <checkout> <id> <file> --tag <tag> --published <YYYY-MM-DD> [--notes <file>] [--signature <file>] [--name <asset>] [--game <version>]
       cargo sunrise-mod registry page <checkout> <id> <sunrise-registry.toml>
       cargo sunrise-mod registry releases --out <releases.json>";

fn binary_checks(sdk_data: Option<&Path>) -> Option<registry::builds::Checks<'_>> {
    sdk_data.map(|sdk_data| registry::builds::Checks {
        sdk_data,
        source: sunrise_native_build::prebuilt::release_source,
    })
}

fn lines(errors: Vec<String>) -> String {
    format!("{} problem(s):\n  {}", errors.len(), errors.join("\n  "))
}

pub fn main(args: &[String]) -> Result<(), String> {
    let command = format!("registry {}", args.first().map_or("", String::as_str));
    // Every subcommand's value options: one a subcommand does not read is refused by `finish`.
    const VALUED: &[&str] = &[
        "--files",
        "--out",
        "--generated",
        "--release",
        "--site",
        "--counts",
        "--releases",
        "--nightly",
        "--nightly-build",
        "--sdk-data",
        "--author",
        "--tag",
        "--published",
        "--notes",
        "--signature",
        "--name",
        "--game",
    ];
    let mut options =
        Options::new(&command, args.get(1..).unwrap_or_default(), VALUED).usage(USAGE);
    match args.first().map(String::as_str) {
        Some("releases") => {
            let out = options.value("--out")?;
            options.finish()?;
            releases::fetch(Path::new(out.ok_or(USAGE)?))
        }
        Some("build") => {
            let (files, out) = (options.value("--files")?, options.value("--out")?);
            let (generated, release) = (options.value("--generated")?, options.value("--release")?);
            let (site, counts) = (options.value("--site")?, options.value("--counts")?);
            let releases = options.value("--releases")?;
            let nightly = options.value("--nightly")?;
            let nightly_build = options.value("--nightly-build")?;
            let sdk_data = options.value("--sdk-data")?.map(Path::new);
            let pos = options.positional();
            options.finish()?;
            let [checkout] = pos[..] else {
                return Err(USAGE.into());
            };
            let (Some(files), Some(out)) = (files, out) else {
                return Err(USAGE.into());
            };
            let generated = generated
                .map(str::to_owned)
                .or_else(|| std::env::var("REGISTRY_GENERATED").ok())
                .unwrap_or_default();
            let releases = read_input(releases, registry::parse_releases)?;
            let nightly = nightly.map(read_json).transpose()?;
            let nightly_build = read_input(nightly_build, registry::parse_nightly_build)?;
            let mut index = registry::build(&Sources {
                registry: Path::new(checkout),
                files: Path::new(files),
                release: release.unwrap_or(RELEASE),
                site: site.unwrap_or(SITE),
                generated,
                releases: releases.as_deref(),
                nightly: nightly.as_ref(),
                binary_checks: binary_checks(sdk_data),
            })
            .map_err(lines)?;
            index.nightly_build = nightly_build;
            fit::publish(&mut index, Path::new(files), sdk_data)?;
            fit::check_binaries(&index)?;
            for problem in uninstallable_versions(&index, |_| true) {
                eprintln!("warning: {problem}");
            }
            // GitHub's download and star counts, which the workflow fetched.
            if let Some(path) = counts {
                registry::count(&mut index, &read_json(path)?);
            }
            let text = serde_json::to_string_pretty(&index).map_err(|e| e.to_string())?;
            std::fs::write(out, text + "\n").map_err(|e| format!("{out}: {e}"))?;
            println!(
                "Wrote {out}: {} project(s), {} taken down",
                index.projects.len(),
                index.revoked.len()
            );
            Ok(())
        }
        Some("review") => {
            let (author, files) = (options.value("--author")?, options.value("--files")?);
            let releases = options.value("--releases")?;
            let sdk_data = options.value("--sdk-data")?.map(Path::new);
            let json = options.switch("--json");
            let pos = options.positional();
            options.finish()?;
            let [base, head] = pos[..] else {
                return Err(USAGE.into());
            };
            let author = author.ok_or(USAGE)?;
            let releases = read_input(releases, registry::parse_releases)?;
            let base = registry::load(Path::new(base)).map_err(lines)?;
            let checkout = Path::new(head);
            let head = registry::load(checkout).map_err(lines)?;
            let mut review = registry::review_checkout(&base, &head, author);
            let files = files.map(Path::new);
            let sources = Sources {
                registry: checkout,
                files: files.unwrap_or(checkout),
                release: RELEASE,
                site: SITE,
                generated: String::new(),
                releases: releases.as_deref(),
                nightly: None,
                binary_checks: binary_checks(sdk_data),
            };
            if let Some(files) = files {
                review.check_native_requirements(
                    &base.entries,
                    &head.entries,
                    files,
                    sdk_data,
                    sunrise_addresses::CARGO_CRATES_IO_SOURCE,
                );
                match registry::candidate(&head, &sources) {
                    Ok(mut index) => {
                        if let Err(error) =
                            fit::verify_binaries(&mut index, sources.files, sdk_data)
                        {
                            review.errors.push(error);
                        }
                        review.notes.extend(registry::takedown_notes(
                            &base.entries,
                            &base.revoked,
                            &index,
                        ));
                        if let Err(error) = index.check() {
                            review.errors.push(error);
                        }
                    }
                    Err(errors) => review.errors.extend(errors),
                }
            } else {
                review
                    .errors
                    .extend(head.check_declarations(sources.releases));
                review.errors.extend(head.check_collections(&sources));
                if registry::new_takedowns(&base.revoked, &head.revoked)
                    .next()
                    .is_some()
                {
                    review.errors.push("to review a takedown, give --files <dir> with the head checkout's packages named by SHA-256".into());
                }
            }
            review.errors.extend(registry::check_screenshot_rule(
                &base.entries,
                &head.entries,
                checkout,
            ));
            if files.is_none() || !review.errors.is_empty() {
                for change in &mut review.build_reviews {
                    if change.status == registry::ReviewStatus::Routine {
                        change.status = registry::ReviewStatus::ReviewRequired;
                        change.reason = "binary checks have not passed".into();
                    }
                }
            }
            if json {
                let json = serde_json::json!({"errors": review.errors, "warnings": review.warnings, "notes": review.notes,
                    "collection_reviews": review.collection_reviews, "nightly_reviews": review.nightly_reviews, "build_reviews": review.build_reviews});
                println!("{json}");
            } else {
                for (label, changes) in [
                    ("collection", review.collection_reviews),
                    ("nightly", review.nightly_reviews),
                    ("builds", review.build_reviews),
                ] {
                    for change in changes {
                        let status = match change.status {
                            registry::ReviewStatus::Routine => "routine",
                            registry::ReviewStatus::ReviewRequired => "review required",
                        };
                        println!("note: {label} {}: {status}: {}", change.id, change.reason);
                    }
                }
                for n in &review.notes {
                    println!("note: {n}");
                }
                for w in &review.warnings {
                    println!("warning: {w}");
                }
                for e in &review.errors {
                    println!("error: {e}");
                }
            }
            if review.errors.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "{} change(s) the registry refuses",
                    review.errors.len()
                ))
            }
        }
        // `check` on each file `head` lists and `base`
        // doesn't: a pull request's new files.
        Some("check") => {
            let (files, json) = (options.value("--files")?, options.switch("--json"));
            let releases = options.value("--releases")?;
            let sdk_data = options.value("--sdk-data")?.map(Path::new);
            let pos = options.positional();
            options.finish()?;
            let [base, head] = pos[..] else {
                return Err(USAGE.into());
            };
            let files = Path::new(files.ok_or(USAGE)?);
            let releases = read_input(releases, registry::parse_releases)?;
            let base_entries = registry::load(Path::new(base)).map_err(lines)?.entries;
            let old: std::collections::HashSet<String> = base_entries
                .iter()
                .flat_map(|e| e.versions.iter().map(|v| v.sha256.clone()))
                .collect();
            let checkout = Path::new(head);
            let head = registry::load(checkout).map_err(lines)?;
            let assembled = registry::candidate(
                &head,
                &Sources {
                    registry: checkout,
                    files,
                    release: RELEASE,
                    site: SITE,
                    generated: String::new(),
                    releases: releases.as_deref(),
                    nightly: None,
                    binary_checks: binary_checks(sdk_data),
                },
            );
            for e in &head.entries {
                for v in e.versions.iter().filter(|v| v.sunrise.is_none()) {
                    eprintln!(
                        "warning: {} {}: legacy version row needs sunrise = \"<stable release tag>\"",
                        e.id, v.version
                    );
                }
            }
            let mut reports = Vec::new();
            let errors = match assembled {
                Err(errors) => errors,
                Ok(mut index) => {
                    // Cache versions are already published. Check only the files added to the
                    // entries by this submission, not unchanged listings harmed by a takedown.
                    let submitted: std::collections::HashSet<_> = head
                        .entries
                        .iter()
                        .flat_map(|e| &e.versions)
                        .filter(|v| !old.contains(&v.sha256))
                        .map(|v| v.sha256.as_str())
                        .collect();
                    let checked = index
                        .check()
                        .and_then(|()| fit::verify_binaries(&mut index, files, sdk_data));
                    let mut errors = match checked {
                        Ok(()) => uninstallable_versions(&index, |v| {
                            submitted.contains(v.file.sha256.as_str())
                        }),
                        Err(error) => vec![error],
                    };
                    for e in &head.entries {
                        for v in e
                            .versions
                            .iter()
                            .filter(|v| submitted.contains(v.sha256.as_str()))
                        {
                            match crate::check::check(
                                &files.join(&v.sha256),
                                e.kind.clone(),
                                e.key.as_deref(),
                                v.signature.as_deref(),
                            ) {
                                Ok(report) if json => reports.push(serde_json::json!({
                                    "project": e.id, "version": v.version, "file": v.file,
                                    "report": report.json(),
                                })),
                                Ok(report) => println!(
                                    "{} {} ({}):\n{}",
                                    e.id,
                                    v.version,
                                    v.file,
                                    report.text().trim_end()
                                ),
                                Err(error) => {
                                    errors.push(format!("{} {}: {error}", e.id, v.version))
                                }
                            }
                        }
                    }
                    errors
                }
            };
            let mut errors = errors;
            errors.extend(registry::check_screenshot_rule(
                &base_entries,
                &head.entries,
                checkout,
            ));
            if json {
                println!(
                    "{}",
                    serde_json::json!({"files": reports, "errors": errors})
                );
            }
            if errors.is_empty() {
                Ok(())
            } else {
                Err(lines(errors))
            }
        }
        // The files CI must download for `build`: `<sha256> <id> <version> <file>`.
        Some("list") => {
            let build_assets = options.switch("--build-assets");
            let files = if build_assets {
                options.value("--files")?.map(Path::new)
            } else {
                None
            };
            let pos = options.positional();
            options.finish()?;
            let [checkout] = pos[..] else {
                return Err(USAGE.into());
            };
            let loaded = registry::load(Path::new(checkout)).map_err(lines)?;
            if build_assets {
                let assets = registry::builds::assets(&loaded.entries, files, &loaded.revoked)?;
                println!(
                    "{}",
                    serde_json::to_string(&assets).map_err(|e| e.to_string())?
                );
                return Ok(());
            }
            for e in &loaded.entries {
                for v in &e.versions {
                    e.source_file_name(v)
                        .map_err(|error| format!("{} {}: {error}", e.id, v.version))?;
                    println!("{} {} {} {}", v.sha256, e.id, v.version, v.file);
                }
            }
            Ok(())
        }
        // A release of a project's repository, downloaded: the checks of a submission, then the
        // version as the index lists it (JSON on stdout), for the registry's release sync.
        Some("release") => {
            let (tag, published) = (options.value("--tag")?, options.value("--published")?);
            let (notes, signature) = (options.value("--notes")?, options.value("--signature")?);
            let (asset, game) = (options.value("--name")?, options.value("--game")?);
            let pos = options.positional();
            options.finish()?;
            let [checkout, id, file] = pos[..] else {
                return Err(USAGE.into());
            };
            let (tag, published) = (tag.ok_or(USAGE)?, published.ok_or(USAGE)?);
            let read = |path: Option<&str>| {
                path.map(|p| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}")))
                    .transpose()
            };
            let (notes, signature) = (read(notes)?, read(signature)?);
            let entry = entry_of(checkout, id)?;
            let path = Path::new(file);
            let cache = registry::Cache::read(Path::new(checkout), id)?;
            let entry = entry.resolved(&cache.page);
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            crate::check::check(
                path,
                entry.kind.clone(),
                entry.key.as_deref(),
                signature.as_deref(),
            )?;
            let version = registry::release_version(
                &entry,
                &registry::Release {
                    tag,
                    published,
                    notes: notes.as_deref().unwrap_or(""),
                    asset: asset.unwrap_or(name),
                    game: game.unwrap_or("26.3"),
                    signature: signature.as_deref(),
                },
                path,
            )?;
            println!(
                "{}",
                serde_json::to_string(&version).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        // The page file of a project's repository, checked against its entry: the page as JSON.
        Some("page") => {
            let pos = options.positional();
            options.finish()?;
            let [checkout, id, file] = pos[..] else {
                return Err(USAGE.into());
            };
            let entry = entry_of(checkout, id)?;
            let text = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
            let page = registry::page(&entry, &text)?;
            println!(
                "{}",
                serde_json::to_string(&page).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

fn read_json<T: DeserializeOwned>(path: &str) -> Result<T, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
}

fn read_input<T>(
    path: Option<&str>,
    parse: impl FnOnce(&str) -> Result<T, String>,
) -> Result<Option<T>, String> {
    path.map(|path| {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        parse(&text).map_err(|e| format!("{path}: {e}"))
    })
    .transpose()
}

/// `mods/<id>.toml` of the checkout, which must name a repository.
fn entry_of(checkout: &str, id: &str) -> Result<registry::Entry, String> {
    registry::load(Path::new(checkout))
        .map_err(lines)?
        .entries
        .into_iter()
        .find(|e| e.id == id && e.repo.is_some())
        .ok_or_else(|| format!("{id}: no entry with a repo"))
}
