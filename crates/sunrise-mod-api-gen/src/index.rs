//! The crates the mod API's signatures name, indexed by [`mod_source::index`].
use mod_source::index::{Binding, target_dependency_active};
pub(crate) use mod_source::index::{Found, Index, ItemId, uses};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
};
use syn::Item;

/// The crates whose items mod API signatures name: crate name and source directory.
pub(crate) const CRATES: &[(&str, &str)] = &[
    ("mod_api", "crates/sunrise-mod-vocab/src"),
    ("sunrise_core", "crates/sunrise-core/src"),
    ("sunrise_mod_runtime", "crates/sunrise-mod-runtime/src"),
];

/// The mod API's type index, read below `root`.
pub(crate) fn game<'a>(root: &'a Path, included: &'a [(&'a str, &'a str)]) -> Index<'a> {
    Index::new(root, included, CRATES).expanding(crate::exports::expand)
}

/// The workspace libraries that the public API cannot treat as opaque third-party types.
pub(crate) struct OutsideSeam {
    crates: HashSet<String>,
    indexed: Vec<WorkspaceCrate>,
    /// Dependency names as Rust spells them, keyed by the indexed crate that declares them.
    dependencies: HashMap<String, HashMap<String, String>>,
    unsupported_edges: HashMap<String, String>,
    default_features: HashMap<String, HashSet<String>>,
}

struct WorkspaceCrate {
    name: String,
    source: String,
    root_file: Option<String>,
}

impl OutsideSeam {
    pub(crate) fn read(root: &Path) -> Result<Self, String> {
        let workspace_manifest = root.join("Cargo.toml");
        crate::rerun_if_changed(&workspace_manifest.display());
        let manifest = crate::replace::manifest(root)?;
        let workspace_members = manifest
            .get("workspace")
            .and_then(|workspace| workspace.get("members"))
            .and_then(toml::Value::as_array)
            .ok_or("Cargo.toml: no workspace.members array")?;
        let workspace_dependencies = manifest
            .get("workspace")
            .and_then(|workspace| workspace.get("dependencies"))
            .and_then(toml::Value::as_table);
        let mut crates = HashSet::new();
        let mut indexed = Vec::new();
        let mut packages = HashMap::new();
        let mut member_manifests = Vec::new();
        for member in workspace_members {
            let member = member
                .as_str()
                .ok_or("Cargo.toml: workspace member is not a path")?;
            let package_path = root.join(member);
            crate::rerun_if_changed(&package_path.join("Cargo.toml").display());
            let package = crate::replace::manifest(&package_path)?;
            let package_name = package
                .get("package")
                .and_then(|package| package.get("name"))
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("{member}/Cargo.toml: no package name"))?;
            // The walk records this crate's re-exported procedural macros as opaque names.
            if package_name == "sunrise-mod-api-macros" {
                continue;
            }
            let crate_name = package
                .get("lib")
                .and_then(|lib| lib.get("name"))
                .or_else(|| package.get("package").and_then(|p| p.get("name")))
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("{member}/Cargo.toml: no crate name"))?;
            let crate_name = crate_name.replace('-', "_");
            // The facade and seam crates are indexed under names their source writes, which
            // differ from some Cargo library names (`mod_api` vs `mod_vocab`).
            let indexed_name = CRATES
                .iter()
                .find(|(_, src)| Path::new(src).parent() == Some(Path::new(member)))
                .map(|(name, _)| (*name).to_owned())
                .or_else(|| {
                    (member == "crates/sunrise-mod-api").then(|| "sunrise_mod_api".to_owned())
                })
                .unwrap_or_else(|| crate_name.clone());
            let seam = member == "crates/sunrise-mod-api"
                || CRATES
                    .iter()
                    .any(|(_, src)| Path::new(src).parent() == Some(Path::new(member)));
            if !seam {
                crates.insert(crate_name.clone());
                let lib_path = package
                    .get("lib")
                    .and_then(|lib| lib.get("path"))
                    .and_then(toml::Value::as_str)
                    .unwrap_or("src/lib.rs");
                let source_file = package_path.join(lib_path);
                if package.get("lib").is_some() || source_file.is_file() {
                    if !source_file.is_file() {
                        return Err(format!(
                            "{member}/Cargo.toml: library source {} is missing",
                            source_file.display()
                        ));
                    }
                    let source = source_file
                        .parent()
                        .and_then(|path| path.strip_prefix(root).ok())
                        .ok_or_else(|| format!("{member}/Cargo.toml: invalid library path"))?
                        .to_string_lossy()
                        .into_owned();
                    let root_file = source_file
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned());
                    indexed.push(WorkspaceCrate {
                        name: crate_name.clone(),
                        source,
                        root_file,
                    });
                    packages.insert(package_name.to_owned(), (crate_name, indexed_name.clone()));
                }
            } else {
                packages.insert(package_name.to_owned(), (crate_name, indexed_name.clone()));
            }
            member_manifests.push((indexed_name, package));
        }

        let mut dependencies = HashMap::new();
        let mut unsupported_edges = HashMap::new();
        let mut default_features = HashMap::new();
        for (owner, package) in member_manifests {
            let mut forwarded = HashMap::new();
            if let Some(features) = package.get("features").and_then(toml::Value::as_table)
                && features.contains_key("default")
            {
                let mut enabled = HashSet::new();
                let mut pending = vec!["default"];
                while let Some(feature) = pending.pop() {
                    if enabled.insert(feature.to_owned())
                        && let Some(edges) = features.get(feature).and_then(toml::Value::as_array)
                    {
                        for edge in edges.iter().filter_map(toml::Value::as_str) {
                            if let Some((dependency, _)) = edge.split_once('/') {
                                forwarded.insert(dependency.trim_end_matches('?'), edge);
                            } else if !edge.starts_with("dep:") {
                                pending.push(edge);
                            }
                        }
                    }
                }
                default_features.insert(owner.clone(), enabled);
            }
            let mut aliases = HashMap::new();
            let mut add = |table: Option<&toml::map::Map<String, toml::Value>>| {
                let Some(table) = table else { return };
                for (alias, declared) in table {
                    let spec =
                        if declared.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
                            workspace_dependencies
                                .and_then(|workspace| workspace.get(alias))
                                .unwrap_or(declared)
                        } else {
                            declared
                        };
                    let package = spec
                        .as_table()
                        .and_then(|table| table.get("package"))
                        .and_then(toml::Value::as_str)
                        .unwrap_or(alias);
                    if let Some((crate_name, indexed_name)) = packages.get(package) {
                        // Without a Cargo rename the extern prelude uses the library target's
                        // name, which need not match the package name or our index name.
                        let written = if alias == package {
                            crate_name.clone()
                        } else {
                            alias.replace('-', "_")
                        };
                        aliases.insert(written, indexed_name.clone());
                        if let Some(edge) = forwarded.get(alias.as_str()) {
                            unsupported_edges.entry(owner.clone()).or_insert_with(|| {
                                format!("dependency `{alias}` (default feature forwards `{edge}`)")
                            });
                        }
                        // Registry crates remain opaque. For indexed dependency source we must
                        // know which items are active; the package's defaults aren't enough here.
                        for config in [spec, declared] {
                            let reason = if config.get("optional").and_then(toml::Value::as_bool)
                                == Some(true)
                            {
                                "optional = true"
                            } else if config
                                .get("default-features")
                                .and_then(toml::Value::as_bool)
                                == Some(false)
                            {
                                "default-features = false"
                            } else if config
                                .get("features")
                                .and_then(toml::Value::as_array)
                                .is_some_and(|features| !features.is_empty())
                            {
                                "features"
                            } else {
                                continue;
                            };
                            unsupported_edges
                                .entry(owner.clone())
                                .or_insert_with(|| format!("dependency `{alias}` ({reason})"));
                        }
                    }
                }
            };
            add(package.get("dependencies").and_then(toml::Value::as_table));
            if let Some(targets) = package.get("target").and_then(toml::Value::as_table) {
                for (selector, target) in targets {
                    if target_dependency_active(selector)? {
                        add(target.get("dependencies").and_then(toml::Value::as_table));
                    }
                }
            }
            if !aliases.is_empty() {
                dependencies.insert(owner, aliases);
            }
        }
        Ok(Self {
            crates,
            indexed,
            dependencies,
            unsupported_edges,
            default_features,
        })
    }

    /// Adds outside workspace crates to the lazy source index so glob imports can be resolved to
    /// actual exported names without treating primitives or prelude types as leaks.
    pub(crate) fn add_to_index(&self, index: &mut Index<'_>) -> Result<(), String> {
        for krate in &self.indexed {
            index.add_crate(&krate.name, &krate.source);
            if let Some(file) = &krate.root_file
                && file != "lib.rs"
            {
                index.set_root_file(&krate.name, file);
            }
        }

        index.add_dependency_aliases(&self.dependencies);
        index.add_default_features(&self.default_features);

        // Resolve every source module reachable from the seam crates in each default build up
        // front. Export lookup can treat an unknown name as absent, but it must not mistake an
        // unreadable module or generated include for a confirmed absence.
        let mut reachable = HashSet::new();
        let mut crates = CRATES
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .chain(std::iter::once("sunrise_mod_api".to_owned()))
            .collect::<VecDeque<_>>();
        while let Some(name) = crates.pop_front() {
            if reachable.insert(name.clone()) {
                if let Some(edge) = self.unsupported_edges.get(&name) {
                    return Err(format!(
                        "{name}: unsupported default API build configuration for {edge}"
                    ));
                }
                crates.extend(
                    self.dependencies
                        .get(&name)
                        .into_iter()
                        .flat_map(HashMap::values)
                        .cloned(),
                );
            }
        }
        for krate in self
            .indexed
            .iter()
            .filter(|krate| reachable.contains(&krate.name))
        {
            let mut pending = VecDeque::from([(krate.name.clone(), Vec::<String>::new())]);
            let mut seen = HashSet::new();
            while let Some((name, path)) = pending.pop_front() {
                if !seen.insert((name.clone(), path.clone())) {
                    continue;
                }
                let module = index.module(&name, &path)?;
                let children = index.modules[module]
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        Item::Mod(child) if index.default_build(module, &child.attrs) => {
                            Some(child.ident.to_string())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                for child in children {
                    let mut child_path = path.clone();
                    child_path.push(child);
                    pending.push_back((name.clone(), child_path));
                }
            }
        }
        Ok(())
    }

    /// The canonical path of a definition owned by a workspace crate outside the seam.
    pub(crate) fn external_item_path(&self, index: &Index<'_>, id: ItemId) -> Option<Vec<String>> {
        self.crates
            .contains(&index.modules[id.module].krate)
            .then(|| index.item_path(id))
            .flatten()
    }

    /// Resolves an indexed source path to an outside workspace crate, following Cargo and `use`
    /// aliases. The returned path uses the owning crate's real Rust library name.
    pub(crate) fn external_path(
        &self,
        index: &mut Index<'_>,
        module: usize,
        path: &[String],
        rooted: bool,
    ) -> Result<Option<Vec<String>>, String> {
        match index.resolve_type_binding(module, path, rooted)? {
            Binding::Found(Found::Item(id)) => Ok(self.external_item_path(index, id)),
            Binding::Found(Found::Module(target)) => Ok(self
                .crates
                .contains(&index.modules[target].krate)
                .then(|| index.module_path(target))),
            Binding::Ambiguous(error) => Err(error),
            Binding::Hidden | Binding::Found(Found::External { .. }) => Ok(None),
            Binding::Missing => {
                // A qualified path into a workspace crate may name a macro-generated item or an
                // associated item, neither of which is in the index. Inspect its longest known
                // prefix without falling back to the extern prelude behind a local binding.
                for end in (1..path.len()).rev() {
                    let (owner, prefix) =
                        match index.resolve_type_binding(module, &path[..end], rooted)? {
                            Binding::Found(Found::Item(id)) => (id.module, index.item_path(id)),
                            Binding::Found(Found::Module(target)) => {
                                (target, Some(index.module_path(target)))
                            }
                            Binding::Ambiguous(error) => return Err(error),
                            Binding::Hidden | Binding::Found(Found::External { .. }) => {
                                return Ok(None);
                            }
                            Binding::Missing => continue,
                        };
                    return Ok(prefix
                        .filter(|_| self.crates.contains(&index.modules[owner].krate))
                        .map(|mut prefix| {
                            prefix.extend_from_slice(&path[end..]);
                            prefix
                        }));
                }
                Ok(None)
            }
        }
    }

    /// A path to an indexed API or local module rather than an outside workspace crate.
    pub(crate) fn internal_module(
        &self,
        index: &mut Index<'_>,
        module: usize,
        path: &[String],
    ) -> Option<usize> {
        let target = index.resolve_module_path(module, path).ok().flatten()?;
        (!self.crates.contains(&index.modules[target].krate)).then_some(target)
    }

    pub(crate) fn error(item: &str, path: &[String]) -> String {
        format!(
            "{item}: `{}` names a workspace crate outside the mod API type index",
            path.join("::")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Index, OutsideSeam};
    use std::fs;

    #[test]
    fn private_workspace_glob_resolves_a_module_exported_through_alias_and_glob() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"consumer\", \"engine\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("consumer/src")).unwrap();
        fs::create_dir_all(root.join("engine/src")).unwrap();
        fs::write(
            root.join("consumer/Cargo.toml"),
            "[package]\nname = \"consumer\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nengine = { package = \"engine-library\", path = \"../engine\" }\n",
        )
        .unwrap();
        fs::write(
            root.join("consumer/src/lib.rs"),
            "use nested::*;\nuse engine::*;\n",
        )
        .unwrap();
        fs::write(
            root.join("engine/Cargo.toml"),
            "[package]\nname = \"engine-library\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[lib]\nname = \"engine_library\"\n",
        )
        .unwrap();
        fs::write(
            root.join("engine/src/lib.rs"),
            "mod private { pub mod bridge { pub mod nested { pub struct Hidden; } } }\npub use private::bridge as bridge_alias;\npub use bridge_alias::*;\n",
        )
        .unwrap();

        let outside = OutsideSeam::read(root).unwrap();
        let mut index = Index::new(root, &[], &[]);
        outside.add_to_index(&mut index).unwrap();
        let consumer = index.module("consumer", &[]).unwrap();
        let path = outside.external_path(
            &mut index,
            consumer,
            &["nested", "Hidden"].map(str::to_owned),
            false,
        );
        let path = path
            .unwrap()
            .expect("the private workspace glob resolves the exported module binding");

        let expected = ["engine_library", "private", "bridge", "nested", "Hidden"];
        assert_eq!(path, expected);
        assert_eq!(
            outside
                .external_path(&mut index, consumer, &["Hidden".to_owned()], false)
                .unwrap(),
            Some(expected.map(str::to_owned).to_vec())
        );
    }

    #[test]
    fn external_lookup_declines_a_cycle_through_named_and_glob_imports() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"consumer\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("consumer/src")).unwrap();
        fs::write(
            root.join("consumer/Cargo.toml"),
            "[package]\nname = \"consumer\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(
            root.join("consumer/src/lib.rs"),
            "use self::cycle as alias;\nuse alias::*;\n",
        )
        .unwrap();

        let outside = OutsideSeam::read(root).unwrap();
        let mut index = Index::new(root, &[], &[]);
        outside.add_to_index(&mut index).unwrap();
        let consumer = index.module("consumer", &[]).unwrap();

        assert_eq!(
            outside
                .external_path(
                    &mut index,
                    consumer,
                    &["alias", "Hidden"].map(str::to_owned),
                    false,
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn two_globbed_modules_with_the_same_name_are_ambiguous() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"consumer\", \"engine\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("consumer/src")).unwrap();
        fs::create_dir_all(root.join("engine/src")).unwrap();
        fs::write(
            root.join("consumer/Cargo.toml"),
            "[package]\nname = \"consumer\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nengine = { package = \"engine-library\", path = \"../engine\" }\n",
        )
        .unwrap();
        fs::write(
            root.join("consumer/src/lib.rs"),
            "use engine::first::*;\nuse engine::second::*;\n",
        )
        .unwrap();
        fs::write(
            root.join("engine/Cargo.toml"),
            "[package]\nname = \"engine-library\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[lib]\nname = \"engine_library\"\n",
        )
        .unwrap();
        fs::write(
            root.join("engine/src/lib.rs"),
            "pub mod first { pub mod nested { pub struct Hidden; } }\npub mod second { pub mod nested { pub struct Hidden; } }\n",
        )
        .unwrap();

        let outside = OutsideSeam::read(root).unwrap();
        let mut index = Index::new(root, &[], &[]);
        outside.add_to_index(&mut index).unwrap();
        let consumer = index.module("consumer", &[]).unwrap();
        let error = outside
            .external_path(
                &mut index,
                consumer,
                &["nested", "Hidden"].map(str::to_owned),
                false,
            )
            .unwrap_err();
        assert!(
            error.contains("ambiguous type-namespace binding `nested`"),
            "{error}"
        );
        assert!(error.contains("engine_library::first::nested"), "{error}");
        assert!(error.contains("engine_library::second::nested"), "{error}");
        assert!(
            error.contains("qualify the intended path explicitly"),
            "{error}"
        );
    }

    #[test]
    fn rooted_path_resolves_root_extern_crate_alias() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"consumer\", \"engine\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("consumer/src")).unwrap();
        fs::create_dir_all(root.join("engine/src")).unwrap();
        fs::write(
            root.join("consumer/Cargo.toml"),
            "[package]\nname = \"consumer\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nengine_library = { package = \"engine-library\", path = \"../engine\" }\n",
        )
        .unwrap();
        fs::write(
            root.join("consumer/src/lib.rs"),
            "extern crate engine_library as engine;\n",
        )
        .unwrap();
        fs::write(
            root.join("engine/Cargo.toml"),
            "[package]\nname = \"engine-library\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[lib]\nname = \"engine_library\"\n",
        )
        .unwrap();
        fs::write(root.join("engine/src/lib.rs"), "pub struct Secret;\n").unwrap();

        let outside = OutsideSeam::read(root).unwrap();
        let mut index = Index::new(root, &[], &[]);
        outside.add_to_index(&mut index).unwrap();
        let consumer = index.module("consumer", &[]).unwrap();

        assert_eq!(
            outside
                .external_path(
                    &mut index,
                    consumer,
                    &["engine", "Secret"].map(str::to_owned),
                    true,
                )
                .unwrap(),
            Some(["engine_library", "Secret"].map(str::to_owned).to_vec())
        );
    }

    #[test]
    fn allowed_type_and_workspace_glob_collision_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"consumer\", \"engine\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("consumer/src")).unwrap();
        fs::create_dir_all(root.join("engine/src")).unwrap();
        fs::create_dir_all(root.join("api/src")).unwrap();
        fs::write(
            root.join("consumer/Cargo.toml"),
            "[package]\nname = \"consumer\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nengine = { package = \"engine-library\", path = \"../engine\" }\n",
        )
        .unwrap();
        fs::write(
            root.join("consumer/src/lib.rs"),
            "use allowed_api::*;\nuse engine::*;\n",
        )
        .unwrap();
        fs::write(
            root.join("engine/Cargo.toml"),
            "[package]\nname = \"engine-library\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[lib]\nname = \"engine_library\"\n",
        )
        .unwrap();
        fs::write(root.join("engine/src/lib.rs"), "pub struct Secret;\n").unwrap();
        fs::write(root.join("api/src/lib.rs"), "pub struct Secret;\n").unwrap();

        let outside = OutsideSeam::read(root).unwrap();
        let mut index = Index::new(root, &[], &[]);
        outside.add_to_index(&mut index).unwrap();
        index.add_crate("allowed_api", "api/src");
        let consumer = index.module("consumer", &[]).unwrap();

        let error = outside
            .external_path(&mut index, consumer, &["Secret".to_owned()], false)
            .unwrap_err();
        assert!(
            error.contains("ambiguous type-namespace binding `Secret`"),
            "{error}"
        );
        assert!(error.contains("allowed_api::Secret"), "{error}");
        assert!(error.contains("engine_library::Secret"), "{error}");
    }
}
