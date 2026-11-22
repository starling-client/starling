//! Registry crates linked once for every mod of a build.
//!
//! Each mod is a `dylib`, and a dylib carries its own copy of every registry `rlib` it links.
//! rustc refuses to link a crate that reaches the same rlib through two dylibs ("cannot satisfy
//! dependencies so `serde` only shows up once"), so a mod depending on two mods that each use
//! `serde` would not link. When two or more mods link packages from outside their own folders
//! (registry crates, or crates the SDK patches in), every one of those packages, as cargo resolved
//! it, goes into one more dylib, [`PACKAGE`], which each of those mods depends on: they then link
//! those crates, and everything they pull in, from that one library instead of carrying copies of
//! their own. Each mod's own dependency entries are left as they are: its requirements, features
//! and the versions its lock pins still decide what cargo resolves.
use super::registry_inventory::{Package, Used};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

/// The shared library's folder, relative to the staging folder.
pub const RELATIVE: &str = "native-shared/registry";
/// Its Cargo package, a local workspace member like the mods.
pub const PACKAGE: &str = "sunrise-registry-shared";
/// Its crate name, which names the built library file.
pub const CRATE: &str = "sunrise_registry_shared";
/// The path a mod's root manifest (`native-api/<id>/Cargo.toml`) names it by.
pub const DEPENDENCY_PATH: &str = "../../native-shared/registry";

/// The packages the shared library links, and the mods that link it.
#[derive(Debug, PartialEq, Eq)]
pub struct Shared {
    crates: BTreeSet<Used>,
    pub logs: BTreeSet<Used>,
    pub users: BTreeSet<String>,
}

impl Shared {
    /// Every package of every user is shared, not only those two mods have in common: a mod using
    /// only `serde_json` and another using only `serde` would otherwise both carry `serde`.
    /// Use the same library for the log facade even in a one-mod build: the forwarder needs to
    /// name that exact crate directly, even if the mod only reaches it through a dependency.
    pub fn select(uses: BTreeMap<String, BTreeSet<Used>>, logs: BTreeSet<Used>) -> Option<Self> {
        let mut crates = logs.clone();
        let mut users = BTreeSet::new();
        for (id, used) in uses {
            if !used.is_empty() || !logs.is_empty() {
                crates.extend(used);
                users.insert(id);
            }
        }
        (users.len() >= 2 || !logs.is_empty()).then_some(Self {
            crates,
            logs,
            users,
        })
    }

    /// Refuses a staged crate that already takes the shared library's package name: cargo would
    /// see two local packages by that name. `stage` lets the refusal name the mod.
    pub fn check_name(&self, packages: &[Package], stage: &Path) -> Result<(), String> {
        let Some(package) = packages
            .iter()
            .find(|package| package.name == PACKAGE && package.source.is_none())
        else {
            return Ok(());
        };
        let path = &package.manifest_path;
        let owner = (|| {
            let folder = path.parent()?.canonicalize().ok()?;
            let mods = stage.join(super::crates::DIR).canonicalize().ok()?;
            let first = folder.strip_prefix(mods).ok()?.components().next()?;
            Some(first.as_os_str().to_string_lossy().into_owned())
        })();
        Err(format!(
            "{}{}: package name {PACKAGE} is reserved for the library the build links the mods' registry crates from; rename the package",
            owner.map(|id| format!("{id}: ")).unwrap_or_default(),
            path.display(),
        ))
    }

    /// Writes the shared library under `out`.
    pub fn write(&self, out: &Path) -> Result<(), String> {
        let dir = out.join(RELATIVE);
        fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
        fs::write(dir.join("Cargo.toml"), self.manifest().to_string())
            .map_err(|e| e.to_string())?;
        fs::write(dir.join("src/lib.rs"), self.source()).map_err(|e| e.to_string())
    }

    /// One entry per resolved package, at exactly its resolved version, with no features of its
    /// own: the build is one workspace, so cargo compiles each package once with the features its
    /// users enable, and this library links that one build.
    fn manifest(&self) -> toml::Table {
        let mut dependencies = toml::Table::new();
        for (position, used) in self.crates.iter().enumerate() {
            dependencies.insert(
                alias(position),
                toml::toml! {
                    package = (used.name.clone())
                    version = (format!("={}", used.version))
                    default-features = false
                }
                .into(),
            );
        }
        let mut manifest = toml::toml! {
            [package]
            name = PACKAGE
            version = "0.0.0"
            edition = "2024"
            [lib]
            crate-type = ["dylib"]
        };
        manifest.insert("dependencies".into(), dependencies.into());
        manifest
    }

    /// Names every crate, so the library links each one (an unnamed dependency is not linked).
    fn source(&self) -> String {
        let mut source = String::from(
            "//! Registry crates the mods of this build link, linked once (written by the launcher).\n",
        );
        let mut installs = String::new();
        for (position, used) in self.crates.iter().enumerate() {
            let alias = alias(position);
            source.push_str(&format!("pub extern crate {alias};\n"));
            if self.logs.contains(used) {
                let forwarder = include_str!("native_log_forwarder.rs.template");
                source.push_str(&format!(
                    "mod log_{position} {{\nuse crate::{alias} as facade;\n{forwarder}\n}}\n"
                ));
                installs.push_str(&format!("log_{position}::install();\n"));
            }
        }
        if !installs.is_empty() {
            source.push_str(&format!("pub fn install_loggers() {{\n{installs}}}\n"));
        }
        source
    }
}

/// Makes the mod whose root manifest and library root these are depend on the shared library and
/// name it, so its registry crates come from there. Refuses a dependency of the mod that already
/// takes the crate name (cargo reads `-` in a dependency key as `_`).
pub fn link(manifest: &mut toml::Table, source: &mut String) -> Result<(), String> {
    let targets = manifest
        .get("target")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|targets| targets.values())
        .filter_map(|target| target.get("dependencies"));
    for table in manifest
        .get("dependencies")
        .into_iter()
        .chain(targets)
        .filter_map(toml::Value::as_table)
    {
        if let Some(key) = table.keys().find(|key| key.replace('-', "_") == CRATE) {
            return Err(format!(
                "dependency {key} takes the crate name {CRATE}, which the build gives the library it links the mods' registry crates from; rename the dependency"
            ));
        }
    }
    manifest
        .entry("dependencies")
        .or_insert_with(|| toml::Table::new().into())
        .as_table_mut()
        .ok_or("invalid dependencies")?
        .insert(
            CRATE.into(),
            toml::toml! { path = DEPENDENCY_PATH package = PACKAGE }.into(),
        );
    source.push_str(&format!("\nextern crate {CRATE} as _;\n"));
    Ok(())
}

fn alias(position: usize) -> String {
    format!("shared_{position}")
}

#[cfg(test)]
mod tests {
    use super::super::registry_inventory::Metadata;
    use super::*;
    use std::path::PathBuf;

    const REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";

    /// A staged build as cargo metadata reports it: local packages get a folder under the stage,
    /// the others are registry crates.
    struct Graph {
        stage: tempfile::TempDir,
        packages: Vec<serde_json::Value>,
        nodes: BTreeMap<String, Vec<serde_json::Value>>,
        members: Vec<String>,
    }

    impl Graph {
        fn new() -> Self {
            Self {
                stage: tempfile::tempdir().unwrap(),
                packages: Vec::new(),
                nodes: BTreeMap::new(),
                members: Vec::new(),
            }
        }

        fn local(&mut self, id: &str, name: &str, folder: &str, kind: &str) -> PathBuf {
            let dir = self.stage.path().join(folder);
            fs::create_dir_all(&dir).unwrap();
            self.packages.push(serde_json::json!({
                "id": id, "name": name, "version": "0.1.0", "source": null, "links": null,
                "manifest_path": dir.join("Cargo.toml"), "targets": [{"kind": [kind]}],
            }));
            self.nodes.entry(id.into()).or_default();
            dir
        }

        fn member(&mut self, id: &str, folder: &str) -> PathBuf {
            self.members.push(id.into());
            self.local(id, id, folder, "lib")
        }

        fn registry(&mut self, id: &str, name: &str, version: &str, kind: &str) {
            self.packages.push(serde_json::json!({
                "id": id, "name": name, "version": version, "source": REGISTRY, "links": null,
                "manifest_path": format!("/registry/{name}-{version}/Cargo.toml"),
                "targets": [{"kind": [kind]}],
            }));
            self.nodes.entry(id.into()).or_default();
        }

        fn edge(&mut self, from: &str, to: &str, kind: Option<&str>, target: Option<&str>) {
            self.nodes.get_mut(from).unwrap().push(serde_json::json!({
                "pkg": to, "dep_kinds": [{"kind": kind, "target": target}],
            }));
        }

        fn metadata(&self) -> Metadata {
            let nodes: Vec<_> = self
                .nodes
                .iter()
                .map(|(id, deps)| serde_json::json!({"id": id, "deps": deps}))
                .collect();
            serde_json::from_value(serde_json::json!({
                "packages": self.packages,
                "workspace_members": self.members,
                "resolve": {"nodes": nodes},
            }))
            .unwrap()
        }

        fn uses(&self, mods: &[(&str, &Path)]) -> BTreeMap<String, BTreeSet<Used>> {
            let mods = mods
                .iter()
                .map(|(id, dir)| ((*id).to_owned(), dir.to_path_buf()))
                .collect();
            self.metadata()
                .runtime_uses(self.stage.path(), &mods)
                .unwrap()
        }
    }

    fn used(name: &str, version: &str) -> Used {
        Used {
            name: name.into(),
            version: version.into(),
        }
    }

    /// Two mods whose requirements differ (`itoa = "*"`, `itoa = "1"`) but resolve to one
    /// package share it once; a genuinely different version is a second crate.
    #[test]
    fn shares_resolved_packages_once_and_keeps_versions_apart() {
        let mut graph = Graph::new();
        let left = graph.member("left", "native-api/left");
        let right = graph.member("right", "native-api/right");
        graph.registry("itoa", "itoa", "1.0.18", "lib");
        graph.registry("rand08", "rand", "0.8.5", "lib");
        graph.registry("rand09", "rand", "0.9.1", "lib");
        graph.edge("left", "itoa", None, None);
        graph.edge("right", "itoa", None, None);
        graph.edge("left", "rand08", None, None);
        graph.edge("right", "rand09", None, None);
        let shared = Shared::select(
            graph.uses(&[("left", &left), ("right", &right)]),
            BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(
            shared.crates,
            BTreeSet::from([
                used("itoa", "1.0.18"),
                used("rand", "0.8.5"),
                used("rand", "0.9.1")
            ])
        );
        let manifest = shared.manifest();
        let dependencies = manifest["dependencies"].as_table().unwrap();
        assert_eq!(dependencies.len(), 3);
        let itoa = dependencies
            .values()
            .find(|dep| dep["package"].as_str() == Some("itoa"))
            .unwrap();
        assert_eq!(itoa["version"].as_str(), Some("=1.0.18"));
        assert_eq!(itoa["default-features"].as_bool(), Some(false));
        assert!(itoa.get("features").is_none());
        let source = shared.source();
        for alias in dependencies.keys() {
            assert!(source.contains(&format!("pub extern crate {alias};")));
        }
    }

    /// What cargo resolved as active for the target is shared, however the mod declared it: a
    /// target-specific dependency, or an optional one a local crate's feature turns on, both
    /// reached through that local crate. Packages the build stages for something else, other
    /// mods and the mod's own crates are not.
    #[test]
    fn shares_the_active_runtime_graph_through_local_crates() {
        let mut graph = Graph::new();
        let left = graph.member("left", "native-api/left");
        let right = graph.member("right", "native-api/right");
        graph.local("helper", "left-helper", "native-api/left/helper", "lib");
        graph.local("api", "mod_api", "crates/mod_api", "lib");
        graph.registry("winapi", "windows-sys", "0.59.0", "lib");
        graph.registry("log", "log", "0.4.22", "lib");
        graph.registry("itoa", "itoa", "1.0.18", "lib");
        graph.edge("left", "helper", None, None);
        graph.edge("left", "api", None, None);
        graph.edge("left", "right", None, None);
        graph.edge("helper", "winapi", None, Some("cfg(windows)"));
        graph.edge("helper", "log", None, None);
        graph.edge("right", "itoa", None, None);
        let uses = graph.uses(&[("left", &left), ("right", &right)]);
        assert_eq!(
            uses["left"],
            BTreeSet::from([used("log", "0.4.22"), used("windows-sys", "0.59.0")])
        );
        assert_eq!(uses["right"], BTreeSet::from([used("itoa", "1.0.18")]));
    }

    /// A host-only helper's dependencies stay out: a local proc macro's, a build script's, a
    /// registry proc macro itself, and dev-dependencies. A mod with only those links nothing.
    #[test]
    fn leaves_host_only_dependencies_out() {
        let mut graph = Graph::new();
        let left = graph.member("left", "native-api/left");
        let right = graph.member("right", "native-api/right");
        graph.local(
            "derive",
            "left-derive",
            "native-api/left/derive",
            "proc-macro",
        );
        graph.registry("syn", "syn", "2.0.90", "lib");
        graph.registry("cc", "cc", "1.2.0", "lib");
        graph.registry("serde_derive", "serde_derive", "1.0.215", "proc-macro");
        graph.registry("itoa", "itoa", "1.0.18", "lib");
        graph.edge("left", "derive", None, None);
        graph.edge("derive", "syn", None, None);
        graph.edge("left", "cc", Some("build"), None);
        graph.edge("left", "serde_derive", None, None);
        graph.edge("left", "itoa", Some("dev"), None);
        graph.edge("right", "itoa", None, None);
        let uses = graph.uses(&[("left", &left), ("right", &right)]);
        assert!(uses["left"].is_empty(), "{:?}", uses["left"]);
        assert_eq!(Shared::select(uses, BTreeSet::new()), None);
    }

    #[test]
    fn stages_only_the_runtime_facade_at_its_resolved_version() {
        let mut graph = Graph::new();
        let root = graph.member("root", "native-api/root");
        graph.registry("helper", "helper", "1.0.0", "lib");
        graph.registry("runtime-log", "log", "0.4.34", "lib");
        graph.registry("dev-log", "log", "0.4.11", "lib");
        graph.local("local-log", "log", "native-api/root/local-log", "lib");
        graph.packages.last_mut().unwrap()["version"] = "0.4.12".into();
        graph.edge("root", "local-log", None, None);
        graph.edge("root", "helper", None, None);
        graph.edge("helper", "runtime-log", None, None);
        graph.edge("root", "dev-log", Some("dev"), None);
        let shared = Shared::select(
            graph.uses(&[("root", &root)]),
            graph.metadata().runtime_logs().unwrap(),
        )
        .unwrap();
        assert_eq!(shared.users, BTreeSet::from(["root".to_owned()]));
        assert_eq!(shared.logs, BTreeSet::from([used("log", "0.4.34")]));
        let manifest = shared.manifest();
        let logs: Vec<_> = manifest["dependencies"]
            .as_table()
            .unwrap()
            .values()
            .filter(|dep| dep["package"].as_str() == Some("log"))
            .collect();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0]["version"].as_str(), Some("=0.4.34"));
    }

    /// A staged crate already named like the shared library is refused, naming its mod.
    #[test]
    fn refuses_a_staged_package_taking_the_reserved_name() {
        let mut graph = Graph::new();
        let left = graph.member("left", "native-api/left");
        let right = graph.member("right", "native-api/right");
        graph.local("taken", PACKAGE, "native-api/left/taken", "lib");
        graph.registry("itoa", "itoa", "1.0.18", "lib");
        graph.edge("left", "taken", None, None);
        graph.edge("taken", "itoa", None, None);
        graph.edge("right", "itoa", None, None);
        let shared = Shared::select(
            graph.uses(&[("left", &left), ("right", &right)]),
            BTreeSet::new(),
        )
        .unwrap();
        let error = shared
            .check_name(&graph.metadata().packages, graph.stage.path())
            .unwrap_err();
        assert!(error.starts_with("left: "), "{error}");
        assert!(error.contains(PACKAGE), "{error}");
    }

    /// The written library parses back as a dylib package, and a linked mod names it; a mod
    /// dependency already taking the crate name, in either spelling, is refused by its key.
    #[test]
    fn writes_the_library_and_links_a_mod() {
        let out = tempfile::tempdir().unwrap();
        let shared = Shared {
            crates: BTreeSet::from([used("itoa", "1.0.18")]),
            logs: BTreeSet::new(),
            users: BTreeSet::from(["left".to_owned(), "right".to_owned()]),
        };
        shared.write(out.path()).unwrap();
        let written: toml::Table = fs::read_to_string(out.path().join(RELATIVE).join("Cargo.toml"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(written["package"]["name"].as_str(), Some(PACKAGE));
        assert_eq!(written["lib"]["crate-type"][0].as_str(), Some("dylib"));
        let mut manifest: toml::Table = "[dependencies]\nitoa = \"1\"".parse().unwrap();
        let mut source = String::from("pub fn f() {}\n");
        link(&mut manifest, &mut source).unwrap();
        assert_eq!(
            manifest["dependencies"][CRATE]["path"].as_str(),
            Some(DEPENDENCY_PATH)
        );
        assert!(source.ends_with(&format!("extern crate {CRATE} as _;\n")));
        for (taken, key) in [
            (
                "[dependencies]\nsunrise-registry-shared = \"1\"",
                "sunrise-registry-shared",
            ),
            (
                "[target.'cfg(unix)'.dependencies]\nsunrise_registry_shared = { path = \"x\" }",
                "sunrise_registry_shared",
            ),
        ] {
            let mut manifest: toml::Table = taken.parse().unwrap();
            let error = link(&mut manifest, &mut String::new()).unwrap_err();
            assert!(error.contains(&format!("dependency {key} ")), "{error}");
        }
    }
}
