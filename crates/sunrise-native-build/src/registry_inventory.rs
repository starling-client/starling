//! Cargo metadata is read without compiling dependencies or executing build-time code.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeLink {
    pub name: String,
    pub version: String,
    pub library: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    pub added_crates: usize,
    pub build_scripts: Vec<String>,
    pub proc_macros: Vec<String>,
}

#[derive(Deserialize)]
pub struct Metadata {
    pub packages: Vec<Package>,
    workspace_members: Vec<String>,
    resolve: Resolve,
}
#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}
#[derive(Deserialize)]
struct Node {
    id: String,
    deps: Vec<Dependency>,
}
#[derive(Deserialize)]
struct Dependency {
    pkg: String,
    dep_kinds: Vec<Kind>,
}
#[derive(Deserialize)]
struct Kind {
    kind: Option<String>,
}

/// A package a mod links at run time, as cargo resolved it: one per name and version (a crate
/// the SDK patches in replaces the registry's, so there is never a second source).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Used {
    pub name: String,
    pub version: String,
}

#[derive(Deserialize)]
pub struct Package {
    id: String,
    pub name: String,
    #[serde(default)]
    pub manifest_path: std::path::PathBuf,
    pub version: String,
    pub source: Option<String>,
    pub links: Option<String>,
    pub targets: Vec<Target>,
}
#[derive(Deserialize)]
pub struct Target {
    pub kind: Vec<String>,
}

impl Metadata {
    fn reachable(&self, runtime: bool) -> Result<BTreeSet<&str>, String> {
        let nodes: std::collections::BTreeMap<_, _> = self
            .resolve
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect();
        let packages: std::collections::BTreeMap<_, _> = self
            .packages
            .iter()
            .map(|package| (package.id.as_str(), package))
            .collect();
        let mut pending: Vec<_> = self.workspace_members.iter().map(String::as_str).collect();
        let mut reachable = BTreeSet::new();
        while let Some(id) = pending.pop() {
            let package = packages.get(id).ok_or("Cargo metadata: missing package")?;
            if runtime
                && package
                    .targets
                    .iter()
                    .any(|target| target.kind.iter().any(|kind| kind == "proc-macro"))
            {
                continue;
            }
            if !reachable.insert(id) {
                continue;
            }
            let node = nodes
                .get(id)
                .ok_or("Cargo metadata: missing resolved node")?;
            pending.extend(
                node.deps
                    .iter()
                    .filter(|dep| {
                        dep.dep_kinds.iter().any(|kind| {
                            if runtime {
                                kind.kind.is_none()
                            } else {
                                kind.kind.as_deref() != Some("dev")
                            }
                        })
                    })
                    .map(|dep| dep.pkg.as_str()),
            );
        }
        Ok(reachable)
    }

    /// The packages each mod links at run time from outside its own folder: the targets of the
    /// normal dependency edges (those active for the build's target and features, as cargo
    /// resolved them) of its root and of the local crates it links, not walking into proc macros,
    /// build scripts' dependencies, other workspace members, or crates staged elsewhere. `mods`
    /// maps each mod to its staged folder under `stage`. A proc-macro target is left out: it runs
    /// in the compiler, it is not linked.
    pub fn runtime_uses(
        &self,
        stage: &std::path::Path,
        mods: &std::collections::BTreeMap<String, std::path::PathBuf>,
    ) -> Result<std::collections::BTreeMap<String, BTreeSet<Used>>, String> {
        let canonical = |path: &std::path::Path| {
            path.canonicalize()
                .map_err(|e| format!("{}: {e}", path.display()))
        };
        let stage = canonical(stage)?;
        let nodes: std::collections::BTreeMap<_, _> = self
            .resolve
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect();
        let mut packages = std::collections::BTreeMap::new();
        let mut folders = std::collections::BTreeMap::new();
        for package in &self.packages {
            packages.insert(package.id.as_str(), package);
            if package.source.is_none() {
                let folder = package
                    .manifest_path
                    .parent()
                    .ok_or("Cargo metadata: invalid manifest path")?;
                folders.insert(package.id.as_str(), canonical(folder)?);
            }
        }
        let members: BTreeSet<&str> = self.workspace_members.iter().map(String::as_str).collect();
        let proc_macro = |package: &Package| {
            package
                .targets
                .iter()
                .any(|target| target.kind.iter().any(|kind| kind == "proc-macro"))
        };
        let mut uses = std::collections::BTreeMap::new();
        for (id, folder) in mods {
            let folder = canonical(folder)?;
            let root = self
                .workspace_members
                .iter()
                .find(|member| folders.get(member.as_str()) == Some(&folder))
                .ok_or_else(|| format!("Cargo metadata: mod {id} is not a workspace member"))?;
            let mut used = BTreeSet::new();
            let mut pending = vec![root.as_str()];
            let mut seen = BTreeSet::new();
            while let Some(current) = pending.pop() {
                if !seen.insert(current) {
                    continue;
                }
                let node = nodes
                    .get(current)
                    .ok_or("Cargo metadata: missing resolved node")?;
                for dep in &node.deps {
                    if !dep.dep_kinds.iter().any(|kind| kind.kind.is_none()) {
                        continue;
                    }
                    let package = packages
                        .get(dep.pkg.as_str())
                        .ok_or("Cargo metadata: missing package")?;
                    if proc_macro(package) || members.contains(dep.pkg.as_str()) {
                        continue;
                    }
                    match folders.get(dep.pkg.as_str()) {
                        // One of this mod's own crates, linked into its library.
                        Some(path) if path.starts_with(&folder) => pending.push(&dep.pkg),
                        // Staged by the build for something else.
                        Some(path) if path.starts_with(&stage) => {}
                        // A registry crate, or one the SDK patches in.
                        _ => {
                            used.insert(Used {
                                name: package.name.clone(),
                                version: package.version.clone(),
                            });
                        }
                    }
                }
            }
            uses.insert(id.clone(), used);
        }
        Ok(uses)
    }

    /// The facade copies in the target graph, including ones reached only through other crates.
    /// The older 0.3 facade forwards to its dependency on 0.4.
    pub fn runtime_logs(&self) -> Result<BTreeSet<Used>, String> {
        let runtime = self.reachable(true)?;
        Ok(self
            .packages
            .iter()
            .filter(|package| {
                package.name == "log"
                    && package.version.starts_with("0.4.")
                    && package.source.as_deref() == Some(sunrise_addresses::CARGO_CRATES_IO_SOURCE)
                    && runtime.contains(package.id.as_str())
            })
            .map(|package| Used {
                name: package.name.clone(),
                version: package.version.clone(),
            })
            .collect())
    }

    pub fn runtime_packages(&self) -> Result<BTreeSet<String>, String> {
        self.reachable(true)
            .map(|ids| ids.into_iter().map(str::to_owned).collect())
    }

    /// What compiling the mods' registry graph adds, judged against `plan`'s game lock: a package
    /// the game locks at the same version is the game's own native library provider. The build
    /// scripts it lists include the mods' own, in the packages under their staged folders `mods`.
    pub fn inventory(
        &self,
        plan: &super::registry_plan::Plan,
        links: &[NativeLink],
        mods: &std::collections::BTreeMap<String, std::path::PathBuf>,
    ) -> Result<Inventory, String> {
        let canonical = |path: &std::path::Path| {
            path.canonicalize()
                .map_err(|e| format!("{}: {e}", path.display()))
        };
        let folders = mods
            .values()
            .map(|folder| canonical(folder))
            .collect::<Result<Vec<_>, _>>()?;
        let reachable = self.reachable(false)?;
        let runtime = self.reachable(true)?;
        let mut scripts = BTreeSet::new();
        let mut macros = BTreeSet::new();
        let mut added = BTreeSet::new();
        for package in &self.packages {
            if !reachable.contains(package.id.as_str()) {
                continue;
            }
            let label = format!("{} {}", package.name, package.version);
            let script = package
                .targets
                .iter()
                .any(|target| target.kind.iter().any(|kind| kind == "custom-build"));
            if package.source.is_none() {
                // A mod's own package, or a crate in its folder: its build script runs too. The
                // game's patched crates and the build's own packages are not the mod's.
                if script {
                    let manifest = canonical(&package.manifest_path)?;
                    if folders.iter().any(|folder| manifest.starts_with(folder)) {
                        scripts.insert(label);
                    }
                }
                continue;
            }
            // The package the game locks at this version links the same library the game does,
            // so it is no second provider. It is still compiled here, build script included.
            let games_own = plan
                .game
                .packages
                .keys()
                .any(|id| id.name == package.name && id.version.to_string() == package.version);
            if let Some(library) = package
                .links
                .as_ref()
                .filter(|_| !games_own && runtime.contains(package.id.as_str()))
                && let Some(existing) = links.iter().find(|claim| &claim.library == library)
            {
                return Err(format!(
                    "Native library {library}: mod package {} {} would compile a second provider of SDK package {} {}; the prebuilt game cannot be relinked with a different provider",
                    package.name, package.version, existing.name, existing.version
                ));
            }
            added.insert(label.clone());
            if script {
                scripts.insert(label.clone());
            }
            if package
                .targets
                .iter()
                .any(|target| target.kind.iter().any(|kind| kind == "proc-macro"))
            {
                macros.insert(label);
            }
        }
        Ok(Inventory {
            added_crates: added.len(),
            build_scripts: scripts.into_iter().collect(),
            proc_macros: macros.into_iter().collect(),
        })
    }
}

pub fn read(
    rust: &dyn crate::rust::Toolchain,
    workspace: &std::path::Path,
) -> Result<Metadata, String> {
    let output = super::cargo(rust, workspace, "metadata")
        .args([
            "--offline",
            "--format-version",
            "1",
            "--filter-platform",
            rust.target(),
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("Cargo metadata: {e}"))
}

/// The digest of the crates each mod's library is compiled with from outside its own files, as
/// cargo resolved this workspace: the tree of compilation units its root reaches through normal
/// and build edges (a build script or proc macro shapes its code too), each by name, version and
/// source with the features enabled on that unit. Under resolver 2 cargo compiles a package once
/// for the target and once for the host, each with its own features, so the digest reads
/// `cargo tree`, which keeps the two apart, not the per-package feature union of
/// `cargo metadata`, which a target feature can join unseen when a build dependency already
/// enabled it. The install set builds as one `--workspace`, which unifies features and shares
/// one lock across it, so the tree is read for the whole workspace the same way: another mod in
/// the set can change these under a mod whose files did not. Another mod's root is left out:
/// the identity names that library by its own identity. A path package is named without its
/// path, which differs between install sets: its files are the mod's own (in the source digest)
/// or the SDK's (in the build). `workspace` is the staged workspace and `mods` maps each mod to
/// its staged folder.
pub fn graphs(
    rust: &dyn crate::rust::Toolchain,
    workspace: &std::path::Path,
    mods: &std::collections::BTreeMap<String, std::path::PathBuf>,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    let canonical = |path: &std::path::Path| {
        path.canonicalize()
            .map_err(|e| format!("{}: {e}", path.display()))
    };
    let mut roots = BTreeMap::new();
    for (id, folder) in mods {
        roots.insert(canonical(folder)?, id.as_str());
    }
    let output = super::cargo(rust, workspace, "tree")
        .args([
            "--offline",
            "--workspace",
            "--target",
            rust.target(),
            "--edges",
            "normal,build",
            "--charset",
            "ascii",
            "--format",
            "{p}|{f}",
            "--prefix",
            "depth",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    let tree = String::from_utf8(output.stdout).map_err(|e| format!("cargo tree: {e}"))?;
    // Each unit's dependencies, from the one place cargo expands it; a later `(*)` repeats it.
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut unit_roots: BTreeMap<&str, &str> = BTreeMap::new();
    let mut parents: Vec<(&str, bool)> = Vec::new();
    for line in tree.lines().filter(|line| !line.is_empty()) {
        let text = line.trim_start_matches(|c: char| c.is_ascii_digit());
        let depth: usize = line[..line.len() - text.len()]
            .parse()
            .map_err(|_| format!("cargo tree: {line}"))?;
        let (unit, repeated) = match text.strip_suffix(" (*)") {
            Some(unit) => (unit, true),
            None => (text, false),
        };
        parents.truncate(depth);
        if let Some(&(parent, true)) = parents.last() {
            children.entry(parent).or_default().push(unit);
        }
        let expands = !repeated && !children.contains_key(unit);
        if expands {
            children.insert(unit, Vec::new());
        }
        parents.push((unit, expands));
        // `{p}` adds a path package's folder in parentheses: `name v1.0.0 (/folder)`.
        let package = unit.split_once('|').map_or(unit, |(package, _)| package);
        if let Some(folder) = package
            .split_once(" (")
            .and_then(|(_, rest)| rest.strip_suffix(')'))
            .filter(|folder| std::path::Path::new(folder).is_absolute())
            && let Ok(folder) = std::path::Path::new(folder).canonicalize()
            && let Some(id) = roots.get(&folder)
        {
            unit_roots.insert(unit, id);
        }
    }
    let mut graphs = BTreeMap::new();
    for (unit, id) in &unit_roots {
        let mut hash = Sha256::new();
        let mut seen = BTreeSet::new();
        let mut pending = vec![(*unit, 0usize)];
        while let Some((current, depth)) = pending.pop() {
            if depth > 0 && unit_roots.contains_key(current) {
                continue;
            }
            let (package, features) = current.split_once('|').unwrap_or((current, ""));
            let mut parts = package.split(" (");
            hash.update(depth.to_string().as_bytes());
            hash.update(parts.next().unwrap_or_default().as_bytes());
            for part in
                parts.filter(|part| !std::path::Path::new(part.trim_end_matches(')')).is_absolute())
            {
                hash.update(b" (");
                hash.update(part.as_bytes());
            }
            hash.update(b"|");
            hash.update(features.as_bytes());
            if !seen.insert(current) {
                hash.update(b" (*)\n");
                continue;
            }
            hash.update(b"\n");
            if let Some(next) = children.get(current) {
                pending.extend(next.iter().rev().map(|child| (*child, depth + 1)));
            }
        }
        graphs.insert(
            (*id).to_owned(),
            hash.finalize().iter().map(|b| format!("{b:02x}")).collect(),
        );
    }
    match mods.keys().find(|id| !graphs.contains_key(*id)) {
        Some(id) => Err(format!("cargo tree: mod {id} is not a workspace member")),
        None => Ok(graphs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn metadata() -> Metadata {
        serde_json::from_value(serde_json::json!({
            "workspace_members": ["root"],
            "resolve": {"nodes": [
                {"id":"root", "deps":[{"pkg":"script", "dep_kinds":[{"kind":null}]}, {"pkg":"macro", "dep_kinds":[{"kind":null}]}, {"pkg":"unused", "dep_kinds":[{"kind":"dev"}]}]},
                {"id":"script", "deps":[]}, {"id":"macro", "deps":[]}, {"id":"unused", "deps":[]}
            ]},
            "packages": [
                {"id":"root", "name":"mod", "version":"1.0.0", "source":null, "links":null, "targets":[{"kind":["lib"]}]},
                {"id":"script", "name":"native-sys", "version":"2.0.0", "source":"registry+https://github.com/rust-lang/crates.io-index", "links":"example", "targets":[{"kind":["lib"]},{"kind":["custom-build"]}]},
                {"id":"macro", "name":"derive-example", "version":"1.0.0", "source":"registry+https://github.com/rust-lang/crates.io-index", "links":null, "targets":[{"kind":["proc-macro"]}]},
                {"id":"unused", "name":"dev-only", "version":"1.0.0", "source":"registry+https://github.com/rust-lang/crates.io-index", "links":"example", "targets":[{"kind":["custom-build"]}]}
            ]
        })).unwrap()
    }
    /// Mods `a` and `b` (on `a`) staged under `stage` as one workspace: `a` links the path
    /// crate `shared` at `version` with `features` for the target, with `shared` and its `std`
    /// feature again as a build dependency, and `tool` at `tool` only as a dev-dependency.
    fn two_mods(
        stage: &std::path::Path,
        version: &str,
        features: &str,
        tool: &str,
        other: &str,
    ) -> std::collections::BTreeMap<String, std::path::PathBuf> {
        let write = |path: std::path::PathBuf, text: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        let package = |name: &str, version: &str| {
            format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2024\"\n")
        };
        write(
            stage.join("Cargo.toml"),
            "[workspace]\nresolver = \"2\"\nmembers = [\"a\", \"b\", \"c\", \"shared\", \"tool\"]\n",
        );
        write(
            stage.join("shared/Cargo.toml"),
            &format!(
                "{}[features]\nstd = []\nalloc = []\n",
                package("shared", version)
            ),
        );
        write(stage.join("tool/Cargo.toml"), &package("tool", tool));
        write(
            stage.join("a/Cargo.toml"),
            &format!(
                "{}[dependencies]\nshared = {{ path = \"../shared\", features = [{features}] }}\n\
                 [build-dependencies]\nshared = {{ path = \"../shared\", features = [\"std\"] }}\n\
                 [dev-dependencies]\ntool = {{ path = \"../tool\" }}\n",
                package("a", "0.1.0")
            ),
        );
        write(stage.join("a/build.rs"), "fn main() {}\n");
        write(
            stage.join("b/Cargo.toml"),
            &format!(
                "{}[dependencies]\na = {{ path = \"../a\" }}\n",
                package("b", "0.1.0")
            ),
        );
        write(
            stage.join("c/Cargo.toml"),
            &format!(
                "{}[dependencies]\nshared = {{ path = \"../shared\", features = [{other}] }}\n",
                package("c", "0.1.0")
            ),
        );
        for name in ["a", "b", "c", "shared", "tool"] {
            write(stage.join(name).join("src/lib.rs"), "");
        }
        ["a", "b"]
            .into_iter()
            .map(|id| (id.to_owned(), stage.join(id)))
            .collect()
    }

    /// A mod's graph digest follows the version and the per-unit features cargo resolved for the
    /// crates it is compiled with, not where the install set is staged or what it only tests
    /// with; a target feature its build dependency already enabled still changes it, as does one
    /// another workspace member enables on the same unit, and a
    /// dependent's own digest leaves its dependency mod out (its identity names that one).
    #[test]
    #[ignore = "daily"]
    fn a_graph_digest_follows_resolved_versions_and_per_unit_features_only() {
        let sysroot = std::process::Command::new("rustc")
            .args(["--print", "sysroot"])
            .output()
            .unwrap();
        assert!(sysroot.status.success());
        let rust = crate::rust::tests::system(std::path::PathBuf::from(
            String::from_utf8(sysroot.stdout).unwrap().trim(),
        ));
        let dir = tempfile::tempdir().unwrap();
        let digest = |stage: &str, version: &str, features: &str, tool: &str, other: &str| {
            let stage = dir.path().join(stage);
            let mods = two_mods(&stage, version, features, tool, other);
            graphs(&rust, &stage, &mods).unwrap()
        };
        let base = digest("one", "1.2.0", "", "1.0.0", "");
        assert_eq!(base, digest("two", "1.2.0", "", "2.0.0", ""));
        let moved = digest("three", "1.1.0", "", "1.0.0", "");
        // The package-level union stays `std` with the build dependency's feature.
        let unified = digest("four", "1.2.0", "\"std\"", "1.0.0", "");
        let added = digest("five", "1.2.0", "\"alloc\"", "1.0.0", "");
        // Another workspace member that enables a feature on the same unit.
        let beside = digest("six", "1.2.0", "", "1.0.0", "\"alloc\"");
        for other in [&moved, &unified, &added, &beside] {
            assert_ne!(base["a"], other["a"]);
            assert_eq!(base["b"], other["b"]);
        }
        assert_ne!(moved["a"], unified["a"]);
        assert_eq!(added["a"], beside["a"]);
        assert_ne!(unified["a"], added["a"]);
    }

    /// A plan whose game locks no registry package.
    fn no_game() -> super::super::registry_plan::Plan {
        super::super::registry_plan::Plan::read("package = []", &[], &[]).unwrap()
    }
    #[test]
    fn consent_counts_only_reachable_build_packages_and_names_executable_code() {
        let inventory = metadata()
            .inventory(&no_game(), &[], &Default::default())
            .unwrap();
        assert_eq!(inventory.added_crates, 2);
        assert_eq!(inventory.build_scripts, ["native-sys 2.0.0"]);
        assert_eq!(inventory.proc_macros, ["derive-example 1.0.0"]);
    }
    #[test]
    fn a_mods_own_build_script_is_listed_with_its_dependencies() {
        let stage = tempfile::tempdir().unwrap();
        let folder = stage.path().join("native-api/scripted");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("Cargo.toml"), "").unwrap();
        let mut metadata = metadata();
        let root = &mut metadata.packages[0];
        root.targets.push(Target {
            kind: vec!["custom-build".into()],
        });
        root.manifest_path = folder.join("Cargo.toml");
        let mods = std::collections::BTreeMap::from([("scripted".to_owned(), folder.clone())]);
        let inventory = metadata.inventory(&no_game(), &[], &mods).unwrap();
        assert_eq!(inventory.build_scripts, ["mod 1.0.0", "native-sys 2.0.0"]);
        // A script of a package outside every mod's folder is not the mod's own.
        let inventory = metadata
            .inventory(&no_game(), &[], &Default::default())
            .unwrap();
        assert_eq!(inventory.build_scripts, ["native-sys 2.0.0"]);
        // Its crates.io dependencies still count alone for the crates added.
        assert_eq!(inventory.added_crates, 2);
    }
    #[test]
    fn host_build_dependencies_do_not_claim_a_game_runtime_native_library() {
        let mut metadata = metadata();
        metadata.resolve.nodes[0].deps[0].dep_kinds[0].kind = Some("build".into());
        let inventory = metadata
            .inventory(
                &no_game(),
                &[NativeLink {
                    name: "sdk-sys".into(),
                    version: "1.0.0".into(),
                    library: "example".into(),
                }],
                &Default::default(),
            )
            .unwrap();
        assert_eq!(inventory.build_scripts, ["native-sys 2.0.0"]);
        assert_eq!(inventory.added_crates, 2);
    }

    #[test]
    fn a_second_native_provider_is_refused_with_both_packages_versions_and_library() {
        let error = metadata()
            .inventory(
                &no_game(),
                &[NativeLink {
                    name: "sdk-sys".into(),
                    version: "1.0.0".into(),
                    library: "example".into(),
                }],
                &Default::default(),
            )
            .unwrap_err();
        for detail in ["native-sys", "2.0.0", "sdk-sys", "1.0.0", "example"] {
            assert!(error.contains(detail), "{error}");
        }
    }

    /// The plan the build makes from the SDK's `sunrise-prebuilt.json`, whose lock holds
    /// `native-sys` at `version` and which names it as the provider of native library `example`.
    fn sdk_plan(version: &str) -> (super::super::registry_plan::Plan, Vec<NativeLink>) {
        let dir = tempfile::tempdir().unwrap();
        let lock = format!(
            "[[package]]\nname = \"native-sys\"\nversion = \"{version}\"\nsource = \"{}\"\nchecksum = \"{}\"\n",
            sunrise_addresses::CARGO_CRATES_IO_SOURCE,
            "0".repeat(64)
        );
        std::fs::write(
            dir.path().join("sunrise-prebuilt.json"),
            serde_json::json!({
                "package_format": sunrise_mod_package::sdk::PACKAGE_FORMAT,
                "commit": "0".repeat(40),
                "rustc": "rustc",
                "target": "x86_64-unknown-linux-gnu",
                "side": "client",
                "registry_lock": lock,
                "registry_patches": {},
                "native_links": [{"name": "native-sys", "version": version, "library": "example"}],
                "facade_crates": [],
                "game_features": [],
                "slices": {"runtime": [], "api": []},
            })
            .to_string(),
        )
        .unwrap();
        let sdk = super::super::prebuilt::identity(dir.path()).unwrap();
        let plan = super::super::registry_plan::Plan::read(&sdk.registry_lock, &[], &[]).unwrap();
        (plan, sdk.native_links)
    }

    #[test]
    fn the_sdks_own_native_provider_at_the_same_version_is_no_second_provider() {
        let (plan, links) = sdk_plan("2.0.0");
        let inventory = metadata()
            .inventory(&plan, &links, &Default::default())
            .unwrap();
        // It is still compiled for the mod, so it still asks for consent.
        assert_eq!(inventory.added_crates, 2);
        assert_eq!(inventory.build_scripts, ["native-sys 2.0.0"]);
    }

    #[test]
    fn the_sdks_native_provider_at_another_version_is_refused() {
        let (plan, links) = sdk_plan("1.0.0");
        let error = metadata()
            .inventory(&plan, &links, &Default::default())
            .unwrap_err();
        for detail in ["native-sys 2.0.0", "native-sys 1.0.0", "example"] {
            assert!(error.contains(detail), "{error}");
        }
    }
}
