//! Local Cargo manifest closure staging and lock-file isolation for native mods.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use super::api;

/// Where the launcher stages the mods' crates.
pub const DIR: &str = "native-api";

pub fn check_game_graph_names(
    projects: &[api::Project<'_>],
    generated_facades: &BTreeMap<String, &str>,
    text: &str,
    game_names: &BTreeSet<String>,
) -> Result<(), String> {
    let lock: toml::Table = text.parse().map_err(|e| format!("Cargo.lock: {e}"))?;
    let packages = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or("Cargo.lock: missing packages")?;
    // Only the owning mod may reach its generated path facade. A resolved registry or git
    // package with the same name is walked like any other crate.
    let mut facades = generated_facades.clone();
    for package in packages
        .iter()
        .filter(|package| package.get("source").is_some())
    {
        if let Some(name) = package.get("name").and_then(toml::Value::as_str) {
            facades.remove(name);
        }
    }
    for project in projects {
        let mod_id = &project.bundle.manifest.id;
        let name = &project.package;
        let mut todo = vec![(name.to_owned(), name.to_owned())];
        let mut seen = BTreeSet::new();
        while let Some((current, via)) = todo.pop() {
            if !seen.insert(current.clone()) {
                continue;
            }
            for package in packages
                .iter()
                .filter(|p| p.get("name").and_then(toml::Value::as_str) == Some(&current))
            {
                for dep in package
                    .get("dependencies")
                    .and_then(toml::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(toml::Value::as_str)
                {
                    let dep = dep.split_whitespace().next().unwrap_or(dep);
                    if facades.get(dep) == Some(&current.as_str()) {
                        continue;
                    }
                    let first = if current == *name {
                        dep.to_owned()
                    } else {
                        via.clone()
                    };
                    if game_names.contains(dep) {
                        return Err(super::api::ProjectError::GameDependency {
                            mod_id: mod_id.clone(),
                            key: format!("{first} -> {dep}"),
                        }
                        .to_string());
                    }
                    todo.push((dep.to_owned(), first));
                }
            }
        }
    }
    Ok(())
}

/// A crate a mod asks for in any Cargo dependency table (the lock includes dev and build deps).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    pub mod_id: String,
    pub name: String,
    pub version: String,
    pub requirement: semver::VersionReq,
}

/// Visit the root manifest already parsed by `Project`, then each reachable local manifest once.
/// `input` is the real staged bundle for a build, or the author's project for lock generation.
/// Remove the SDK dependencies, emit local manifests through `write_local`, and return the
/// rewritten root manifest plus the registry requests for the lock check.
pub(crate) fn rewrite_closure(
    project: &api::Project<'_>,
    input: &Path,
    mut write_local: impl FnMut(&Path, &mut toml::Table) -> Result<(), String>,
) -> Result<(toml::Table, Vec<Wanted>), String> {
    struct LocalEdge {
        manifest: PathBuf,
        key: String,
        package: String,
    }
    let mod_id = &project.bundle.manifest.id;
    let mut wanted = Vec::new();
    let mut root = project.cargo.clone();
    let mut seen = BTreeMap::<PathBuf, String>::new();
    let mut pending = vec![(PathBuf::new(), None::<LocalEdge>)];
    while let Some((relative, edge)) = pending.pop() {
        let dir = input.join(&relative);
        let manifest_path = dir.join("Cargo.toml");
        let parsed_dependencies;
        let mut parsed_manifest;
        let (manifest, dependencies) = if let Some(edge) = edge {
            let wrong_package = || {
                format!(
                    "{}: path dependency {} has the wrong package name (expected {} in {})",
                    edge.manifest.display(),
                    edge.key,
                    edge.package,
                    manifest_path.display()
                )
            };
            if let Some(package) = seen.get(&relative) {
                if package != &edge.package {
                    return Err(wrong_package());
                }
                continue;
            }
            let metadata = fs::symlink_metadata(&manifest_path)
                .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
            if is_link_or_junction(&metadata) || !metadata.is_file() {
                return Err(format!(
                    "{}: dependency manifest must be an ordinary file",
                    manifest_path.display()
                ));
            }
            parsed_manifest = api::read(&manifest_path)?;
            if parsed_manifest
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(toml::Value::as_str)
                != Some(edge.package.as_str())
            {
                return Err(wrong_package());
            }
            seen.insert(relative.clone(), edge.package);
            api::check_source_overrides(&parsed_manifest)
                .map_err(|error| format!("{}: {error}", manifest_path.display()))?;
            parsed_dependencies = api::parse_dependencies(mod_id, &parsed_manifest)
                .map_err(|error| format!("{}: {error}", manifest_path.display()))?;
            (&mut parsed_manifest, parsed_dependencies.as_slice())
        } else {
            seen.insert(relative.clone(), project.package.clone());
            (&mut root, project.dependencies.as_slice())
        };
        if manifest.contains_key("workspace") {
            return Err(format!(
                "{}: [workspace] is refused",
                manifest_path.display()
            ));
        }
        for dep in dependencies.iter() {
            match &dep.source {
                api::CargoSource::Sdk { .. } => {
                    dep.table_mut(manifest).remove(&dep.key);
                }
                api::CargoSource::Local { package, path } => {
                    let inside = local_path(input, &relative, path).map_err(|error| {
                        format!(
                            "{}: dependency {}: {error}",
                            manifest_path.display(),
                            dep.key
                        )
                    })?;
                    pending.push((
                        inside,
                        Some(LocalEdge {
                            manifest: manifest_path.clone(),
                            key: dep.key.clone(),
                            package: package.clone(),
                        }),
                    ));
                }
                api::CargoSource::Registry { package, version } => {
                    let version = version.as_ref().ok_or_else(|| {
                        format!(
                            "{}: registry dependency {} must declare a version",
                            manifest_path.display(),
                            dep.key
                        )
                    })?;
                    let requirement = semver::VersionReq::parse(version).map_err(|error| {
                        format!(
                            "{}: dependency {}: {error}",
                            manifest_path.display(),
                            dep.key
                        )
                    })?;
                    wanted.push(Wanted {
                        mod_id: mod_id.clone(),
                        name: package.clone(),
                        version: version.clone(),
                        requirement,
                    });
                }
            }
        }
        if !relative.as_os_str().is_empty() {
            write_local(&relative, manifest)?;
        }
    }
    wanted.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    Ok((root, wanted))
}

/// Resolve a path within one mod without following links or traversing outside its root.
pub(super) fn local_path(root: &Path, relative: &Path, path: &str) -> Result<PathBuf, String> {
    if path.contains('\\') {
        return Err("dependency path must use portable separators".into());
    }
    let mut inside = PathBuf::new();
    for component in relative.components().chain(Path::new(path).components()) {
        match component {
            Component::Normal(part) => {
                // A Windows drive prefix can parse as Normal in the middle of a path,
                // then replace the path so far when pushed on its own. Reject it on every host.
                if matches!(part.as_encoded_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic())
                    || matches!(
                        Path::new(part).components().next(),
                        Some(Component::Prefix(_) | Component::RootDir)
                    )
                {
                    return Err("path outside the mod is refused".into());
                }
                inside.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir if inside.pop() => {}
            _ => return Err("path outside the mod is refused".into()),
        }
    }
    let mut checked = fs::canonicalize(root).map_err(|e| e.to_string())?;
    if !checked.join(&inside).starts_with(&checked) {
        return Err("path outside the mod is refused".into());
    }
    for part in inside.components() {
        checked.push(part);
        let meta = fs::symlink_metadata(&checked).map_err(|e| e.to_string())?;
        if is_link_or_junction(&meta) {
            return Err("linked dependency path is refused".into());
        }
    }
    Ok(inside)
}

/// `symlink_metadata` reports links without following them. Windows junctions are reparse points
/// as well, so reject the reparse attribute even if the standard file type is not a symlink.
pub fn is_link_or_junction(metadata: &fs::Metadata) -> bool {
    let link = metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        link || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    link
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &Path, body: &str) -> super::super::Bundle {
        write_fixture(dir, body);
        super::super::bundle_in(dir).unwrap().unwrap()
    }

    fn write_fixture(dir: &Path, body: &str) {
        super::super::tests::write_bundle(dir, "bake", "");
        fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname='bake-engine'\nversion='0.1.0'\n[dependencies]\nsunrise-mod-api={{path='.sunrise/sdk/api/crates/sunrise-mod-api'}}\n{body}"),
        )
        .unwrap();
        // A path dependency's folder must be in the package, so the helper the bodies name exists
        // before the bundle is read; each test then writes the manifest it wants into it.
        write_manifest(&dir.join("src/helper"), "helper", "");
    }

    fn write_manifest(dir: &Path, package: &str, body: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname={package:?}\nversion='0.1.0'\n{body}"),
        )
        .unwrap();
    }

    #[test]
    fn real_staging_rewrites_a_shared_local_manifest_and_collects_its_pins() {
        let input = tempfile::tempdir().unwrap();
        let build = tempfile::tempdir().unwrap();
        let bundle = fixture(
            input.path(),
            "helper={path='src/helper'}\nregex='1'\n[target.'cfg(unix)'.dev_dependencies]\nalt={path='src/./helper',package='helper'}\n",
        );
        let helper = "src/helper";
        let child_manifest = "[build-dependencies]\nfmt={package='itoa',version='=1.0.18'}\n[target.'cfg(windows)'.dependencies]\nsdk={package='sunrise-mod-api',path='.sunrise/sdk/api/crates/sunrise-mod-api'}\n";
        write_manifest(&input.path().join(helper), "helper", child_manifest);
        let project = api::Project::read(&bundle).unwrap();
        let staged = build.path().join(DIR).join(&bundle.manifest.id);
        super::super::copy_bundle(&bundle, &staged).unwrap();
        let (root, wanted) = rewrite_closure(&project, &staged, |local, manifest| {
            fs::write(staged.join(local).join("Cargo.toml"), manifest.to_string())
                .map_err(|e| e.to_string())
        })
        .unwrap();
        assert!(root["dependencies"].get("sunrise-mod-api").is_none());
        assert_eq!(
            root["dependencies"]["helper"]["path"].as_str(),
            Some("src/helper")
        );
        let local = api::read(&staged.join(helper).join("Cargo.toml")).unwrap();
        assert!(
            local["target"]["cfg(windows)"]["dependencies"]
                .get("sdk")
                .is_none()
        );
        assert_eq!(
            local["build-dependencies"]["fmt"]["version"].as_str(),
            Some("=1.0.18")
        );
        let pins: Vec<_> = wanted
            .iter()
            .map(|w| (w.mod_id.as_str(), w.name.as_str(), w.version.as_str()))
            .collect();
        assert_eq!(pins, [("bake", "itoa", "=1.0.18"), ("bake", "regex", "1")]);
        let original = api::read(&input.path().join(helper).join("Cargo.toml")).unwrap();
        assert!(
            original["target"]["cfg(windows)"]["dependencies"]
                .get("sdk")
                .is_some()
        );
    }

    #[test]
    fn authored_paths_cannot_escape_or_name_a_different_local_package() {
        for (body, expected) in [
            ("bad={path='../other'}", "leaves the package"),
            ("bad={path='/absolute'}", "leaves the package"),
            ("bad={path=7}", "dependency path must be a string"),
            ("bad={path='src/not_a_crate'}", "Is a directory"),
            (
                "bad={path='src/helper',package='different'}\n[target.'cfg(unix)'.dependencies]\nhelper={path='src/helper'}",
                "path dependency bad has the wrong package name",
            ),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let input = temp.path().join("bake");
            write_fixture(&input, body);
            write_manifest(&temp.path().join("other"), "bad", "");
            write_manifest(&input.join("src/helper"), "helper", "");
            fs::create_dir_all(input.join("src/not_a_crate/Cargo.toml")).unwrap();
            // Reading the package refuses a path that leaves it or names a folder it lacks.
            let result = super::super::bundle_in(&input)
                .map(|bundle| bundle.unwrap())
                .and_then(|bundle| {
                    let project = api::Project::read(&bundle).map_err(|e| e.to_string())?;
                    rewrite_closure(&project, &input, |_, _| Ok(()))
                });
            let error = result.unwrap_err();
            assert!(error.contains(expected), "{body}: {error}");
        }
    }

    #[test]
    fn nested_manifests_cannot_override_the_builds_sources() {
        let input = tempfile::tempdir().unwrap();
        let bundle = fixture(input.path(), "helper={path='src/helper'}");
        let helper = input.path().join("src/helper");
        let project = api::Project::read(&bundle).unwrap();
        for (body, expected) in [
            ("[patch.crates-io]\nitoa={path='../outside'}", "[patch]"),
            ("[replace]\n'itoa:1.0.0'={path='../outside'}", "[replace]"),
            ("[workspace]", "[workspace]"),
            (
                "[dependencies]\nsdk={package='sunrise-mod-api',path='../../crates/sunrise-mod-api'}",
                "E_SDK_SOURCE",
            ),
            (
                "[target.'cfg(unix)'.build-dependencies]\ngame={package='sunrise-client',version='1'}",
                "E_GAME_DEPENDENCY",
            ),
        ] {
            write_manifest(&helper, "helper", body);
            let error = rewrite_closure(&project, input.path(), |_, _| Ok(())).unwrap_err();
            assert!(error.contains(expected), "{body}: {error}");
        }
        write_manifest(&helper, "helper", "");
        fs::create_dir(helper.join(".cargo")).unwrap();
        fs::write(helper.join(".cargo/config"), "").unwrap();
        let staged = tempfile::tempdir().unwrap();
        super::super::copy_bundle(&bundle, staged.path()).unwrap();
        assert!(!staged.path().join("src/helper/.cargo/config").exists());
        rewrite_closure(&project, staged.path(), |_, _| Ok(())).unwrap();
    }

    #[test]
    fn path_sources_cannot_be_combined_with_other_selectors() {
        for (selector, value) in [
            ("git", "'https://example.invalid/repo'"),
            ("registry", "'crates-io'"),
            ("registry-index", "'https://example.invalid/index'"),
            ("branch", "'main'"),
            ("rev", "'0123456789abcdef'"),
            ("tag", "'v1.0.0'"),
            ("workspace", "true"),
        ] {
            let input = tempfile::tempdir().unwrap();
            let bundle = fixture(input.path(), "helper={path='src/helper'}");
            write_manifest(
                &input.path().join("src/helper"),
                "helper",
                &format!(
                    "[target.'cfg(unix)'.dev-dependencies]\nprobe={{path='nested', {selector}={value}}}"
                ),
            );
            let project = api::Project::read(&bundle).unwrap();
            let error = rewrite_closure(&project, input.path(), |_, _| Ok(())).unwrap_err();
            assert!(
                error.contains(&format!("{selector} source is refused")),
                "{error}"
            );
        }
    }

    #[test]
    fn a_registry_dependency_cannot_reach_the_game_via_the_mod_api_package_name() {
        let input = tempfile::tempdir().unwrap();
        let bundle = fixture(input.path(), "parent='1'");
        let projects = [api::Project::read(&bundle).unwrap()];
        let lock = r#"
            [[package]]
            name = 'bake-engine'
            dependencies = ['parent']
            [[package]]
            name = 'parent'
            dependencies = ['sunrise-mod-api 1.0.0']
            [[package]]
            name = 'sunrise-mod-api'
            version = '1.0.0'
            source = 'registry+https://github.com/rust-lang/crates.io-index'
        "#;
        let game_names = BTreeSet::from(["sunrise-mod-api".to_owned()]);
        let error =
            check_game_graph_names(&projects, &BTreeMap::new(), lock, &game_names).unwrap_err();
        assert!(error.contains("parent -> sunrise-mod-api"), "{error}");
    }

    #[test]
    fn only_the_owning_mods_generated_facade_is_exempt_from_the_game_graph_check() {
        let input = tempfile::tempdir().unwrap();
        let bundle = fixture(input.path(), "");
        let projects = [api::Project::read(&bundle).unwrap()];
        let facades = BTreeMap::from([("sunrise-internals-bake".to_owned(), "bake-engine")]);
        let game_names = BTreeSet::from(["sunrise-client".to_owned()]);
        let lock = r#"
            [[package]]
            name = 'bake-engine'
            dependencies = ['sunrise-internals-bake']
            [[package]]
            name = 'sunrise-internals-bake'
            dependencies = ['sunrise-client']
        "#;
        check_game_graph_names(&projects, &facades, lock, &game_names).unwrap();
        let foreign = lock.replace("dependencies = ['sunrise-internals-bake']", "dependencies = ['parent']\n[[package]]\nname='parent'\ndependencies=['sunrise-internals-bake']");
        let registry =
            format!("{lock}\nsource='registry+https://github.com/rust-lang/crates.io-index'");
        for lock in [&foreign, &registry] {
            let error = check_game_graph_names(&projects, &facades, lock, &game_names).unwrap_err();
            assert!(error.contains("-> sunrise-client"), "{error}");
        }
    }
}
