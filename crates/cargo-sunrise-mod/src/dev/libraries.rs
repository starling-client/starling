//! Select the add-on's library mods before invoking the native build: the ones given with
//! `--with`, then the signed v2 registry for every other required one.
use crate::registry_source::{Registry, write_files};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use sunrise_mod_package::{
    Manifest, Package,
    dependencies::{self, Dependency},
    index::{
        Kind,
        native::Bundle,
        v2::{Project, Release},
    },
    manifest::Side,
};

struct LocalSource {
    path: PathBuf,
    package: Package,
}

pub(super) struct Libraries {
    local: BTreeMap<String, LocalSource>,
    /// Read only when a required library isn't given with `--with`.
    registry: Option<Registry>,
    side: Side,
}

/// What fills one library slot.
enum Source<'a> {
    Local(&'a LocalSource),
    Registry(Taken<'a>),
}

/// A registry release whose package was verified and whose manifest was read.
struct Taken<'a> {
    project: &'a Project,
    release: &'a Release,
    archive: PathBuf,
    manifest: Manifest,
    /// A native library's source files; none for a content library.
    files: Option<BTreeMap<String, Vec<u8>>>,
}

impl Source<'_> {
    fn manifest(&self) -> &Manifest {
        match self {
            Source::Local(source) => &source.package.manifest,
            Source::Registry(taken) => &taken.manifest,
        }
    }
}

/// Whether a registry release can fill its slot: `Err` is why it can't, and the next older
/// release is tried; the outer `Err` stops the build.
type Take<'a> = Result<Result<Taken<'a>, String>, String>;

impl Libraries {
    pub(super) fn read(
        project: &Manifest,
        paths: &[&Path],
        side: Side,
        cache: &Path,
    ) -> Result<Self, String> {
        let mut local = BTreeMap::new();
        for path in paths {
            let path =
                dunce::canonicalize(path).map_err(|e| format!("--with {}: {e}", path.display()))?;
            if !path.is_dir() && path.extension().and_then(|s| s.to_str()) != Some("smod") {
                return Err(format!(
                    "--with {}: expected a mod folder or .smod",
                    path.display()
                ));
            }
            let package =
                Package::read(&path).map_err(|e| format!("--with {}: {e}", path.display()))?;
            let id = &package.manifest.id;
            if *id == project.id || local.contains_key(id) {
                return Err(format!(
                    "duplicate mod id {id} in --with sources and the project"
                ));
            }
            local.insert(id.clone(), LocalSource { path, package });
        }
        // Walk libraries supplied with --with, including explicit optional libraries. Only an
        // unsupplied required edge needs the registry.
        let mut todo = vec![project];
        let mut seen = BTreeSet::new();
        let mut registry = None;
        'local_closure: while let Some(manifest) = todo.pop() {
            for dep in dependencies::from_manifest(&manifest.dependencies, side) {
                if let Some(source) = local.get(&dep.id) {
                    if seen.insert(source.package.manifest.id.as_str()) {
                        todo.push(&source.package.manifest);
                    }
                } else if !dep.optional && dep.id != project.id {
                    registry = Some(Registry::read(cache).map_err(|error| format!(
                        "{} needs {} {}. Supply the library with --with <path> or use the Sunrise mod registry. Could not read the mod registry: {error}",
                        manifest.id, dep.id, dep.requirement
                    ))?);
                    break 'local_closure;
                }
            }
        }
        Ok(Self {
            local,
            registry,
            side,
        })
    }

    /// Fill every slot the project's required edges reach. A library given with `--with` takes
    /// its slot whatever the registry holds; an optional edge never adds a registry library but
    /// still constrains a library that is present. Each registry slot takes the newest release
    /// that satisfies the requirements known when it is reached and that `take` accepts; a later
    /// requirement it doesn't satisfy is reported, not solved.
    fn select<'a>(
        &'a self,
        project: &Manifest,
        mut take: impl FnMut(&'a Project, &'a Release) -> Take<'a>,
    ) -> Result<BTreeMap<String, Source<'a>>, String> {
        let mut chosen = BTreeMap::new();
        let mut needs: Vec<(String, Dependency)> = Vec::new();
        let mut todo = vec![(
            project.id.clone(),
            dependencies::from_manifest(&project.dependencies, self.side),
        )];
        while let Some((owner, edges)) = todo.pop() {
            for dep in edges {
                let (id, optional) = (dep.id.clone(), dep.optional);
                needs.push((owner.clone(), dep));
                if id == project.id || chosen.contains_key(&id) {
                    continue;
                }
                let source = if let Some(local) = self.local.get(&id) {
                    Source::Local(local)
                } else if optional {
                    continue;
                } else {
                    let wanted: Vec<_> = needs.iter().filter(|(_, d)| d.id == id).collect();
                    Source::Registry(self.registry_release(&id, &wanted, &mut take)?)
                };
                todo.push((
                    id.clone(),
                    dependencies::from_manifest(&source.manifest().dependencies, self.side),
                ));
                chosen.insert(id, source);
            }
        }
        for (owner, dep) in &needs {
            let version = match chosen.get(&dep.id) {
                Some(source) => &source.manifest().version,
                None if dep.id == project.id => &project.version,
                None => continue,
            };
            if !dep.requirement.matches(version) {
                return Err(format!(
                    "{owner} needs {id} {}, but {id} {version} was selected. Supply a matching library with --with <path>.",
                    dep.requirement,
                    id = dep.id,
                ));
            }
        }
        Ok(chosen)
    }

    fn registry_release<'a>(
        &'a self,
        id: &str,
        wanted: &[&(String, Dependency)],
        take: &mut impl FnMut(&'a Project, &'a Release) -> Take<'a>,
    ) -> Result<Taken<'a>, String> {
        let needs = wanted
            .iter()
            .map(|(owner, dep)| format!("{owner} needs {id} {}", dep.requirement))
            .collect::<Vec<_>>()
            .join("; ");
        let index = &self
            .registry
            .as_ref()
            .ok_or_else(|| format!("{needs}. Supply the library with --with <path>."))?
            .index;
        // A slot is the in-game ID; the provider is never picked by the index's order.
        let providers: Vec<_> = index
            .projects
            .iter()
            .filter(|p| {
                p.mod_id == id
                    && matches!(p.kind, Kind::Mod | Kind::NativeMod)
                    && index.revoked(&p.id, "").is_none()
            })
            .collect();
        let project = match providers[..] {
            [project] => project,
            [] => {
                return Err(format!(
                    "{needs}: no project in the Sunrise mod registry provides it. Supply the library with --with <path>."
                ));
            }
            _ => {
                return Err(format!(
                    "{needs}: several projects in the Sunrise mod registry provide it ({}). Supply the one to use with --with <path>.",
                    providers
                        .iter()
                        .map(|p| format!("{} from {}", p.id, p.repository.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        };
        let mut releases: Vec<_> = project
            .releases
            .iter()
            .filter(|release| {
                wanted
                    .iter()
                    .all(|(_, dep)| dep.requirement.matches(&release.version))
                    && index.revoked(&project.id, &release.file.sha256).is_none()
            })
            .collect();
        releases.sort_by(|a, b| b.version.cmp_precedence(&a.version));
        let mut rejected = Vec::new();
        for release in releases {
            match take(project, release)? {
                Ok(taken) => return Ok(taken),
                Err(why) => rejected.push(format!("{id} {}: {why}", release.version)),
            }
        }
        Err(std::iter::once(format!(
            "{needs}: no release of registry project {} satisfies it and fits this build. Supply the library with --with <path>.",
            project.id
        ))
        .chain(rejected)
        .collect::<Vec<_>>()
        .join("\n"))
    }

    pub(super) fn stage(
        &self,
        bundle: &sunrise_native_build::Bundle,
        sdk: &super::NativeSdk,
        cache: &Path,
        root: &Path,
        release: bool,
    ) -> Result<BTreeMap<String, PathBuf>, String> {
        let mut build_data = None;
        let game = &bundle.native().game;
        let selected = self.select(&bundle.manifest, |project, row| {
            let registry = self.registry.as_ref().expect("a registry release");
            let archive = registry.package(cache, project, row)?;
            let failed = |e: String| format!("{}: {e}", archive.display());
            let (manifest, files) = if project.kind == Kind::NativeMod {
                let source = Bundle::read(&archive).map_err(failed)?;
                (source.manifest, Some(source.files))
            } else {
                let package = Package::read(&archive).map_err(failed)?;
                if package.is_native() {
                    return Ok(Err(
                        "the registry lists content, but its package is native".into()
                    ));
                }
                (package.manifest, None)
            };
            if manifest.id != project.mod_id || manifest.version != row.version {
                return Ok(Err(format!(
                    "its package is {} {}",
                    manifest.id, manifest.version
                )));
            }
            if let (Some(files), Some(native)) = (&files, &manifest.native) {
                if native.game != *game {
                    return Ok(Err(format!("it is made for Minecraft {}", native.game)));
                }
                if native.sunrise != sdk.index.build.commit {
                    let candidate = tempfile::tempdir_in(cache).map_err(|e| e.to_string())?;
                    write_files(candidate.path(), files)?;
                    if let Err(why) = check_fit(
                        &bundle.dir,
                        sdk,
                        candidate.path(),
                        &manifest,
                        &mut build_data,
                        release.then_some(root),
                    ) {
                        return Ok(Err(why));
                    }
                }
            }
            Ok(Ok(Taken {
                project,
                release: row,
                archive,
                manifest,
                files,
            }))
        })?;
        let mut origins = BTreeMap::new();
        for (id, source) in selected {
            let source = match source {
                Source::Local(source) => source,
                Source::Registry(taken) => {
                    match taken.files {
                        Some(files) => write_files(&root.join(super::stage_name(&id)), files)?,
                        None => super::copy(
                            &taken.archive,
                            &root.join(format!("{}.smod", super::stage_name(&id))),
                        )?,
                    }
                    println!(
                        "Using library {id} {} from the Sunrise mod registry (project {})",
                        taken.release.version, taken.project.id
                    );
                    continue;
                }
            };
            if let Some(native) = &source.package.manifest.native {
                let into = root.join(super::stage_name(&id));
                let files = if source.path.is_dir() {
                    origins.insert(id.clone(), source.path.clone());
                    directory_files(&source.package, &source.path)?
                } else {
                    Bundle::read(&source.path)?.files
                };
                write_files(&into, files)?;
                if native.sunrise != sdk.index.build.commit {
                    check_fit(
                        &bundle.dir,
                        sdk,
                        &into,
                        &source.package.manifest,
                        &mut build_data,
                        release.then_some(root),
                    )?;
                }
            } else {
                let into = root.join(format!("{}.smod", super::stage_name(&id)));
                if source.path.is_dir() {
                    std::fs::write(&into, Package::pack_directory(&source.path)?)
                        .map_err(|e| format!("{}: {e}", into.display()))?;
                } else {
                    source.package.read_runtime(&source.path)?;
                    super::copy(&source.path, &into)?;
                }
            }
            println!(
                "Using library {id} {} from {}",
                source.package.manifest.version,
                source.path.display()
            );
        }
        Ok(origins)
    }
}

/// Published SDKs compare the library's recorded uses, just as the install path does. Keep the
/// original pin in the source snapshot; only the SDK used for this particular build changes.
/// `private` is a release's private build folder, where its build data is fetched.
fn check_fit(
    project: &Path,
    sdk: &super::NativeSdk,
    root: &Path,
    manifest: &Manifest,
    build_data: &mut Option<PathBuf>,
    private: Option<&Path>,
) -> Result<(), String> {
    use sunrise_native_build::{
        check::{self, Unlisted, Verdict},
        contract::CONTRACT,
    };
    let build = &sdk.index.build.commit;
    let pin = &manifest.native.as_ref().expect("native library").sunrise;
    let id = &manifest.id;
    let mut plan = check::Plan {
        contract: CONTRACT,
        mods: root.to_owned(),
        build: build.clone(),
        surface: Some(sdk.surface()),
        build_data: build_data.clone(),
    };
    let refused =
        |why: String| format!("library {id} pins {pin} and cannot be built for {build}: {why}");
    let judge = |plan: &check::Plan| {
        check::check(plan)?
            .mods
            .into_iter()
            .next()
            .ok_or_else(|| "native-check returned no verdict for this library".to_owned())
    };
    let mut row = judge(&plan).map_err(refused)?;
    if row.needs == Some("build-data") {
        // A release judges with build data fetched from the signed SDK into its private build
        // folder, never the project's own cache, which the checkout can edit.
        *build_data = Some(match private {
            Some(dir) => {
                let staging = dir.join("sdk-data");
                crate::sdk_fetch::release_data(
                    &crate::sdk_fetch::Base::from_env(),
                    &crate::sdk_fetch::keys(),
                    build,
                    &staging,
                    true,
                )
                .map_err(refused)?;
                staging.join(build).join(crate::sdk_fetch::BUILD_DATA)
            }
            None => crate::sdk_fetch::build_data(project, build).map_err(refused)?,
        });
        plan.build_data = build_data.clone();
        row = judge(&plan).map_err(refused)?;
    }
    match row.verdict {
        Some(verdict) if verdict.usable() => (),
        Some(Verdict::DoesNotFit { misfits, .. }) => {
            return Err(refused(
                misfits
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }
        Some(Verdict::NoList { why, hooks, .. }) => {
            let detail = match why {
                Unlisted::Missing => "it has no uses list".into(),
                Unlisted::Unreadable => "its uses list is unreadable".into(),
                Unlisted::NewerSchema => "its uses list requires a newer tool".into(),
                Unlisted::OtherBuild => "its uses list is for another build".into(),
                Unlisted::HookVersions => {
                    format!("its uses list omits versions for {}", hooks.join(", "))
                }
                Unlisted::GameCode => "its uses list omits declared game code".into(),
            };
            return Err(refused(format!(
                "{detail}; build or repin the library against this SDK"
            )));
        }
        Some(Verdict::Exact { why, .. }) => {
            return Err(refused(format!(
                "it declares {why}, which needs the exact SDK"
            )));
        }
        _ => {
            return Err(refused(row.refusal.unwrap_or_else(|| {
                format!(
                    "native-check needs {}",
                    row.needs.unwrap_or("a fit verdict")
                )
            })));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

fn directory_files(package: &Package, dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut files = package.directory_files(dir)?;
    let uses = mod_surface::uses::USES_FILE;
    match crate::bounded::read_bytes(
        &dir.join(uses),
        mod_surface::uses::MAX_USES_BYTES as u64,
        crate::bounded::Symlinks::Refuse,
    ) {
        Ok(table) => {
            files.insert(uses.into(), table);
        }
        Err(error) if error.is_not_found() => (),
        Err(error) => return Err(error.into()),
    }
    Ok(files)
}
