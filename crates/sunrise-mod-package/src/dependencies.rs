//! Resolve bundle dependencies without touching the staged workspace.
use crate::manifest::Side;
mod notices;
pub use notices::{Notice, NoticeKind};
use semver::{Version, VersionReq};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidServerReason {
    NativeDeclaresServedBy,
    MissingHost,
    HostIsContentPackage,
    HostDoesNotServeContent,
    SideNotCovered { host: Side, content: Side },
}

impl fmt::Display for InvalidServerReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NativeDeclaresServedBy => f.write_str("a [native] mod cannot declare served_by"),
            Self::MissingHost => f.write_str("serving native mod is not installed"),
            Self::HostIsContentPackage => {
                f.write_str("target is a content package, not a [native] mod")
            }
            Self::HostDoesNotServeContent => {
                f.write_str("target does not declare [native] serves_content = true")
            }
            Self::SideNotCovered { host, content } => write!(
                f,
                "side {:?} is not covered by host side {:?}",
                side_name(*content),
                side_name(*host)
            ),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Manifest {
        owner: String,
        target: String,
        reason: String,
    },
    Duplicate(String),
    Missing {
        owner: String,
        target: String,
        note: String,
    },
    CargoConflict {
        owner: String,
        target: String,
    },
    Version {
        owner: String,
        target: String,
        version: String,
        requirement: String,
        note: String,
    },
    Cycle {
        mods: Vec<String>,
    },
    Incompatible {
        owner: String,
        target: String,
        version: String,
        /// The owner's reason and link on one logical line, or empty.
        note: String,
    },
    InvalidServer {
        owner: String,
        target: String,
        reason: InvalidServerReason,
    },
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest {
                owner,
                target,
                reason,
            } => write!(f, "{owner}: dependency {target}: {reason}"),
            Self::Duplicate(id) => write!(f, "Duplicate mod id {id:?}"),
            Self::Missing {
                owner,
                target,
                note,
            } => write!(f, "{owner} requires missing mod {target}{note}"),
            Self::CargoConflict { owner, target } => {
                write!(
                    f,
                    "{owner}: dependency {target} conflicts with its Cargo manifest"
                )
            }
            Self::Version {
                owner,
                target,
                version,
                requirement,
                note,
            } => write!(
                f,
                "{owner} requires {target} {requirement}, but installed version is {version}{note}"
            ),
            Self::Cycle { mods } => write!(f, "Mod dependency cycle: {}", mods.join(" -> ")),
            Self::Incompatible {
                owner,
                target,
                version,
                note,
            } => write!(f, "{owner} is incompatible with {target} {version}{note}"),
            Self::InvalidServer {
                owner,
                target,
                reason,
            } => write!(f, "{owner}: served_by {target}: {reason}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Dependency {
    pub id: String,
    pub requirement: VersionReq,
    pub optional: bool,
    pub side: Side,
    pub reason: Option<String>,
    pub url: Option<String>,
    pub order: mod_vocab::DependencyOrder,
}

pub fn from_manifest(
    values: &BTreeMap<String, crate::manifest::Dependency>,
    running: Side,
) -> Vec<Dependency> {
    values
        .iter()
        .filter_map(|(id, value)| {
            let details = value.details();
            if running != Side::Both && details.side != Side::Both && details.side != running {
                return None;
            }
            Some(Dependency {
                id: id.clone(),
                requirement: details.version,
                optional: details.optional,
                side: details.side,
                reason: details.reason,
                url: details.url,
                order: details.order,
            })
        })
        .collect()
}

#[cfg(test)]
fn parse(owner: &str, value: Option<&toml::Value>) -> Result<Vec<Dependency>, Error> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let manifest = format!(
        "id='test'\nname='Test'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n{}\n[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
        toml::to_string(&toml::Table::from_iter([(
            "dependencies".into(),
            value.clone()
        )]))
        .unwrap(),
        "a".repeat(40),
        "b".repeat(40)
    );
    let manifest =
        crate::manifest::Manifest::parse(&manifest).map_err(|reason| Error::Manifest {
            owner: owner.into(),
            target: "[dependencies]".into(),
            reason,
        })?;
    Ok(from_manifest(
        &manifest.dependencies,
        crate::manifest::Side::Both,
    ))
}

/// A package in the logical dependency graph or the native build's link graph.
#[derive(Clone)]
pub struct Mod<'a> {
    pub id: &'a str,
    pub version: &'a Version,
    /// `true` for a native package. Content-only packages have no library to link.
    pub code: bool,
    pub dependencies: &'a [Dependency],
}

#[derive(Debug, Default)]
pub struct Resolution {
    /// Original bundle indices, in dispatch order.
    pub order: Vec<usize>,
    /// What the load rules let through: a discouraged pair, an override applied.
    pub warnings: Vec<Notice>,
    /// Direct logical dependency rows remaining after side and override filtering.
    pub effective: mod_vocab::__private::EffectiveDependencies,
}

/// The dependency rows a standalone build can wire to the projects it was given.
/// Keep absent optional rows so native staging can still declare their empty
/// feature. An absent required row is checked against the installation, not
/// Cargo metadata; an actual reference still needs a compiled library.
pub fn build_dependencies(
    manifest: &crate::Manifest,
    side: Side,
    supplied: impl Fn(&str) -> bool,
) -> Vec<Dependency> {
    from_manifest(&manifest.dependencies, side)
        .into_iter()
        .filter(|dep| dep.optional || supplied(&dep.id))
        .collect()
}

/// Validate and order a mixed set of code and content declarations.
pub fn resolve(mods: &[Mod<'_>]) -> Result<Resolution, Error> {
    resolve_all(mods, &[], &[], &Overrides::default(), false)
}

/// Physical library order, independent of manifest validation/dispatch overrides.
///
/// Only edges between supplied native packages enter this projection. Content-only
/// packages have no library to link. Version ranges and dispatch order are logical
/// rules; Cargo and the native linker enforce actual imports and ABI requirements.
pub fn resolve_links(mods: &[Mod<'_>]) -> Result<Resolution, Error> {
    resolve_all(mods, &[], &[], &Overrides::default(), true)
}

/// Validate the applicable installed set with its instance overrides, then return
/// logical dispatch order and structured notices for both affected Mods rows.
pub fn resolve_with_rules(
    mods: &[Mod<'_>],
    rules: &[Vec<Rule>],
    overrides: &Overrides,
) -> Result<Resolution, Error> {
    resolve_all(mods, rules, &[], overrides, false)
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Client => "client",
        Side::Both => "both",
        Side::Server => "server",
        _ => "unknown",
    }
}

fn side_covers(host: Side, content: Side) -> bool {
    match content {
        Side::Both => host == Side::Both,
        Side::Client => matches!(host, Side::Client | Side::Both),
        Side::Server => matches!(host, Side::Server | Side::Both),
        _ => false,
    }
}

/// Validate every `served_by` declaration against the installed manifest set.
///
/// Each `served_by` target must be an installed `[native]` package with
/// `serves_content = true`, and its declared `side` must cover the content package's `side`.
pub fn validate_served_by(manifests: &[&crate::Manifest]) -> Result<(), Error> {
    let mut by_id = HashMap::with_capacity(manifests.len());
    for manifest in manifests {
        if by_id.insert(manifest.id.as_str(), *manifest).is_some() {
            return Err(Error::Duplicate(manifest.id.clone()));
        }
    }
    for manifest in manifests {
        let Some(target_id) = manifest.served_by.as_deref() else {
            continue;
        };
        if manifest.native.is_some() {
            return Err(Error::InvalidServer {
                owner: manifest.id.clone(),
                target: target_id.to_owned(),
                reason: InvalidServerReason::NativeDeclaresServedBy,
            });
        }
        let Some(host) = by_id.get(target_id).copied() else {
            return Err(Error::InvalidServer {
                owner: manifest.id.clone(),
                target: target_id.to_owned(),
                reason: InvalidServerReason::MissingHost,
            });
        };
        let Some(native) = &host.native else {
            return Err(Error::InvalidServer {
                owner: manifest.id.clone(),
                target: target_id.to_owned(),
                reason: InvalidServerReason::HostIsContentPackage,
            });
        };
        if !native.serves_content {
            return Err(Error::InvalidServer {
                owner: manifest.id.clone(),
                target: target_id.to_owned(),
                reason: InvalidServerReason::HostDoesNotServeContent,
            });
        }
        if !side_covers(host.side, manifest.side) {
            return Err(Error::InvalidServer {
                owner: manifest.id.clone(),
                target: target_id.to_owned(),
                reason: InvalidServerReason::SideNotCovered {
                    host: host.side,
                    content: manifest.side,
                },
            });
        }
    }
    Ok(())
}

/// Resolve one applicable installed manifest snapshot with one shared policy for
/// native and content packages. Indices in the result refer to `manifests`.
pub fn resolve_manifests(
    manifests: &[&crate::Manifest],
    running: Side,
    overrides: &Overrides,
) -> Result<Resolution, Error> {
    validate_served_by(manifests)?;
    let id_index: HashMap<&str, usize> = manifests
        .iter()
        .enumerate()
        .map(|(index, manifest)| (manifest.id.as_str(), index))
        .collect();
    let served_edges: Vec<(usize, usize)> = manifests
        .iter()
        .enumerate()
        .filter_map(|(index, manifest)| {
            let host = manifest.served_by.as_deref()?;
            Some((index, id_index[host]))
        })
        .collect();
    let declared: Vec<_> = manifests
        .iter()
        .map(|manifest| from_manifest(&manifest.dependencies, running))
        .collect();
    let inputs: Vec<_> = manifests
        .iter()
        .zip(&declared)
        .map(|(manifest, dependencies)| Mod {
            id: &manifest.id,
            version: &manifest.version,
            code: manifest.native.is_some(),
            dependencies,
        })
        .collect();
    let rules: Vec<_> = manifests
        .iter()
        .map(|manifest| rules(manifest, running))
        .collect();
    resolve_all(&inputs, &rules, &served_edges, overrides, false)
}

fn diagnostic_note(reason: Option<&str>, url: Option<&str>) -> String {
    // Diagnostics remain one logical line; only the Mods-page renderer adds line breaks.
    let reason = reason.map(|text| text.trim_end().replace(['\r', '\n'], " "));
    let url = url.map(|text| text.replace(['\r', '\n'], " "));
    match (reason.as_deref(), url.as_deref()) {
        (Some(reason), Some(url)) => format!(" ({reason} {url})"),
        (Some(text), None) | (None, Some(text)) => format!(" ({text})"),
        (None, None) => String::new(),
    }
}

/// What a load rule asks of another mod.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    /// `[incompatible]`: refuse to start beside it.
    Incompatible,
    /// `[discouraged]`: start beside it with a warning.
    Discouraged,
    /// `load_before`: run before it when it is loaded.
    LoadBefore,
}

/// One of a mod's load rules about another mod, on the running side.
#[derive(Clone, Debug)]
pub struct Rule {
    pub kind: RuleKind,
    pub id: String,
    /// The other mod's versions the rule matches.
    pub requirement: VersionReq,
    /// Author-supplied text and link, retained separately from diagnostic layout.
    pub reason: Option<String>,
    pub url: Option<String>,
}

/// `manifest`'s load rules that hold on `running`, the client or the dedicated server.
pub fn rules(manifest: &crate::Manifest, running: Side) -> Vec<Rule> {
    fn declared<'a>(
        kind: RuleKind,
        table: &'a BTreeMap<String, crate::manifest::Rule>,
        running: Side,
    ) -> impl Iterator<Item = Rule> + 'a {
        table.iter().filter_map(move |(id, rule)| {
            let rule = rule.details();
            (running == Side::Both || rule.side == Side::Both || rule.side == running).then(|| {
                Rule {
                    kind,
                    id: id.clone(),
                    requirement: rule.version,
                    reason: rule.reason,
                    url: rule.url,
                }
            })
        })
    }
    declared(RuleKind::Incompatible, &manifest.incompatible, running)
        .chain(declared(
            RuleKind::Discouraged,
            &manifest.discouraged,
            running,
        ))
        .chain(manifest.load_before.iter().map(|id| Rule {
            kind: RuleKind::LoadBefore,
            id: id.clone(),
            requirement: VersionReq::STAR,
            reason: None,
            url: None,
        }))
        .collect()
}

/// What an instance's overrides file does to one mod's rule about another (NeoForge's
/// `FMLConfig.DependencyOverride`).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Override {
    /// Ignore the rule.
    Drop,
    /// Load beside an incompatible mod, with a warning.
    Discourage,
}

/// The overrides file's name, in the instance's `sunrise-mods` folder.
pub const OVERRIDES: &str = "dependency-overrides.toml";

/// An instance's overrides of the mods' load rules: `[owner]` tables of `target = "drop"` or
/// `"discourage"`.
#[derive(Debug, Default, Deserialize)]
#[serde(transparent)]
pub struct Overrides(BTreeMap<String, BTreeMap<String, Override>>);

impl Overrides {
    /// The overrides file in `mods_dir`; none when there is no file.
    pub fn read(mods_dir: &Path) -> Result<Self, String> {
        let path = mods_dir.join(OVERRIDES);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let overrides: Self =
                    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
                for (owner, targets) in &overrides.0 {
                    for id in std::iter::once(owner).chain(targets.keys()) {
                        if !crate::manifest::valid_mod_id(id) {
                            return Err(format!(
                                "{}: invalid dependency override mod id {id:?}",
                                path.display()
                            ));
                        }
                    }
                }
                Ok(overrides)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn get(&self, owner: &str, target: &str) -> Option<Override> {
        self.0.get(owner)?.get(target).copied()
    }
}

fn resolve_all(
    mods: &[Mod<'_>],
    rules: &[Vec<Rule>],
    mandatory_edges: &[(usize, usize)],
    overrides: &Overrides,
    links: bool,
) -> Result<Resolution, Error> {
    let mut ids = HashMap::new();
    for (index, item) in mods.iter().enumerate() {
        if ids.insert(item.id, index).is_some() {
            return Err(Error::Duplicate(item.id.into()));
        }
    }
    let mut edges = vec![Vec::new(); mods.len()];
    let mut warnings = Vec::new();
    let mut effective = mod_vocab::__private::EffectiveDependencies::default();
    for (index, item) in mods.iter().enumerate() {
        if links && !item.code {
            continue;
        }
        for dep in item.dependencies {
            let target = ids.get(dep.id.as_str()).copied();
            let note = diagnostic_note(dep.reason.as_deref(), dep.url.as_deref());
            if !links && let Some(action) = overrides.get(item.id, &dep.id) {
                warnings.push(Notice {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                    version: target.map(|i| mods[i].version.to_string()),
                    reason: dep.reason.clone(),
                    url: dep.url.clone(),
                    kind: match action {
                        Override::Drop => NoticeKind::OverrideDropped,
                        Override::Discourage => NoticeKind::OverrideDiscouraged,
                    },
                });
                // No validation, dispatch edge or Part permission remains. The
                // physical resolution still enforces library requirements.
                continue;
            }
            if !links {
                effective.insert(item.id, &dep.id, dep.optional);
            }
            let Some(target) = target else {
                if dep.optional {
                    continue;
                }
                return Err(Error::Missing {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                    note,
                });
            };
            let other = &mods[target];
            if links && !other.code {
                continue;
            }
            if !links
                && (crate::manifest::is_unresolved(other.version)
                    || !dep.requirement.matches(other.version))
            {
                return Err(Error::Version {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                    version: other.version.to_string(),
                    requirement: dep.requirement.to_string(),
                    note,
                });
            }
            use mod_vocab::DependencyOrder;
            if links || dep.order == DependencyOrder::After {
                edges[index].push(target);
            } else if dep.order == DependencyOrder::Before {
                edges[target].push(index);
            } else if dep.order != DependencyOrder::None {
                return Err(Error::Manifest {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                    reason: "unsupported dependency order".into(),
                });
            }
        }
    }
    for (index, item) in mods.iter().enumerate() {
        for rule in rules.get(index).into_iter().flatten() {
            let target = ids.get(rule.id.as_str()).copied();
            let mut kind = rule.kind;
            if let Some(action) = overrides.get(item.id, &rule.id) {
                warnings.push(Notice {
                    owner: item.id.into(),
                    target: rule.id.clone(),
                    version: target.map(|i| mods[i].version.to_string()),
                    reason: rule.reason.clone(),
                    url: rule.url.clone(),
                    kind: match action {
                        Override::Drop => NoticeKind::OverrideDropped,
                        Override::Discourage => NoticeKind::OverrideDiscouraged,
                    },
                });
                if action == Override::Drop || kind == RuleKind::LoadBefore {
                    continue;
                }
                kind = RuleKind::Discouraged;
            }
            let Some(target) = target else {
                continue;
            };
            if kind == RuleKind::LoadBefore {
                edges[target].push(index);
                continue;
            }
            let other = &mods[target];
            if !rule.requirement.matches(other.version) {
                continue;
            }
            if kind == RuleKind::Incompatible {
                return Err(Error::Incompatible {
                    owner: item.id.into(),
                    target: rule.id.clone(),
                    version: other.version.to_string(),
                    note: diagnostic_note(rule.reason.as_deref(), rule.url.as_deref()),
                });
            }
            warnings.push(Notice {
                owner: item.id.into(),
                target: rule.id.clone(),
                version: Some(other.version.to_string()),
                reason: rule.reason.clone(),
                url: rule.url.clone(),
                kind: NoticeKind::Discouraged,
            });
        }
    }
    if !links {
        for &(dependent, target) in mandatory_edges {
            edges[dependent].push(target);
        }
    }
    let mut done = vec![false; mods.len()];
    let mut order = Vec::with_capacity(mods.len());
    while order.len() < mods.len() {
        let next = (0..mods.len()).find(|&i| !done[i] && edges[i].iter().all(|&d| done[d]));
        let Some(next) = next else {
            // Follow remaining edges until a node repeats, reporting the actual cycle only.
            let mut seen = BTreeMap::new();
            let mut path: Vec<usize> = Vec::new();
            let mut current = done.iter().position(|done| !done).expect("unfinished node");
            loop {
                if let Some(&start) = seen.get(&current) {
                    let mut cycle: Vec<String> = path[start..]
                        .iter()
                        .map(|&i| mods[i].id.to_owned())
                        .collect();
                    cycle.push(mods[current].id.to_owned());
                    return Err(Error::Cycle { mods: cycle });
                }
                seen.insert(current, path.len());
                path.push(current);
                current = *edges[current]
                    .iter()
                    .find(|&&i| !done[i])
                    .expect("unfinished dependency");
            }
        };
        done[next] = true;
        order.push(next);
    }
    Ok(Resolution {
        order,
        warnings,
        effective,
    })
}

/// A target's staged Cargo package, or none for an absent optional mod.
pub struct Package<'a> {
    pub dependency: &'a Dependency,
    pub package: Option<&'a str>,
}

/// Add bundle edges to the dependent package. Returns features to enable on the game edge.
pub fn rewrite(
    owner: &str,
    manifest: &mut toml::Table,
    packages: &[Package<'_>],
) -> Result<Vec<String>, Error> {
    let mut updated = manifest.clone();
    let mut enabled = Vec::new();
    for target in packages {
        let id = &target.dependency.id;
        let normalized = id.replace('-', "_");
        let conflict = || Error::CargoConflict {
            owner: owner.into(),
            target: id.clone(),
        };
        let dependencies = updated
            .entry("dependencies")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(conflict)?;
        if dependencies
            .keys()
            .any(|key| key.replace('-', "_") == normalized)
        {
            return Err(conflict());
        }
        if let Some(package) = target.package {
            let mut entry = toml::Table::new();
            entry.insert("path".into(), format!("../{id}").into());
            entry.insert("package".into(), package.into());
            if target.dependency.optional {
                entry.insert("optional".into(), true.into());
            }
            dependencies.insert(id.clone(), toml::Value::Table(entry));
        } else if !target.dependency.optional {
            return Err(Error::Missing {
                owner: owner.into(),
                target: id.clone(),
                note: diagnostic_note(
                    target.dependency.reason.as_deref(),
                    target.dependency.url.as_deref(),
                ),
            });
        }
        if target.dependency.optional {
            let features = updated
                .entry("features")
                .or_insert_with(|| toml::Value::Table(toml::Table::new()))
                .as_table_mut()
                .ok_or_else(conflict)?;
            if features.contains_key(id)
                || features.values().any(|value| {
                    value.as_array().is_some_and(|entries| {
                        entries.iter().any(|entry| {
                            entry.as_str().is_some_and(|s| {
                                s == id
                                    || s == format!("dep:{id}")
                                    || s.starts_with(&format!("{id}/"))
                                    || s.starts_with(&format!("{id}?/"))
                            })
                        })
                    })
                })
            {
                return Err(conflict());
            }
            let entries = if target.package.is_some() {
                enabled.push(id.clone());
                vec![toml::Value::String(format!("dep:{id}"))]
            } else {
                Vec::new()
            };
            features.insert(id.clone(), toml::Value::Array(entries));
        }
    }
    *manifest = updated;
    Ok(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_preserve_author_punctuation_without_forced_line_breaks() {
        assert_eq!(
            diagnostic_note(
                Some("Keep the complete author sentence.  "),
                Some("https://example.test/details")
            ),
            " (Keep the complete author sentence. https://example.test/details)"
        );
        let notice = Notice {
            owner: "owner".into(),
            target: "target".into(),
            version: Some("1.0.0".into()),
            reason: Some("First sentence.\nSecond sentence.".into()),
            url: Some("https://example.test/details".into()),
            kind: NoticeKind::Discouraged,
        };
        assert_eq!(
            notice.to_string(),
            "owner works badly with target 1.0.0 (First sentence. Second sentence. https://example.test/details)"
        );
    }
    #[test]
    fn diagnostic_url_line_breaks_do_not_reach_refusals_or_logs() {
        let url = "https://example.test/\r\ndetails";
        let refusal = Error::Incompatible {
            owner: "owner".into(),
            target: "target".into(),
            version: "1".into(),
            note: diagnostic_note(Some("Reason."), Some(url)),
        };
        assert_eq!(
            refusal.to_string(),
            "owner is incompatible with target 1 (Reason. https://example.test/  details)"
        );
        let notice = Notice {
            owner: "owner".into(),
            target: "target".into(),
            version: Some("1".into()),
            reason: Some("Reason.".into()),
            url: Some(url.into()),
            kind: NoticeKind::Discouraged,
        };
        assert_eq!(
            notice.to_string(),
            "owner works badly with target 1 (Reason. https://example.test/  details)"
        );
    }

    fn deps(text: &str) -> Vec<Dependency> {
        let value: toml::Table = text.parse().unwrap();
        parse("user", value.get("dependencies")).unwrap()
    }
    fn item<'a>(id: &'a str, version: &'a str, dependencies: &'a [Dependency]) -> Mod<'a> {
        Mod {
            id,
            version: Box::leak(Box::new(crate::manifest::parse_version(version).unwrap())),
            code: true,
            dependencies,
        }
    }
    #[test]
    fn an_unresolved_version_satisfies_no_requirement_but_a_real_zero_version_does() {
        for requirement in ["*", "=0.0.0", "<1", "^0.0.0"] {
            let wanted = deps(&format!("[dependencies]\nbase={{version='{requirement}'}}"));
            let user = item("user", "1", &wanted);
            let placeholder = item("base", "0.0.0+unresolved.2-rc", &[]);
            assert!(
                matches!(
                    resolve(&[user.clone(), placeholder]),
                    Err(Error::Version { .. })
                ),
                "{requirement}"
            );
            let real = item("base", "0.0.0+nightly", &[]);
            assert!(resolve(&[user, real]).is_ok(), "{requirement}");
        }
    }

    #[test]
    fn a_client_only_required_dependency_does_not_refuse_a_dedicated_installation() {
        let manifest = crate::Manifest::parse("id='server'\nname='Server'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n[dependencies]\nclient={version='^1',side='client',order='none',reason='Client integration',url='https://example.test/client'}").unwrap();
        let dedicated = from_manifest(&manifest.dependencies, Side::Server);
        assert_eq!(
            resolve(&[item("server", "1", &dedicated)]).unwrap().order,
            [0]
        );
        let client = from_manifest(&manifest.dependencies, Side::Client);
        assert_eq!(client.len(), 1);
        assert_eq!(client[0].reason.as_deref(), Some("Client integration"));
        assert_eq!(
            client[0].url.as_deref(),
            Some("https://example.test/client")
        );
        assert_eq!(client[0].order, mod_vocab::DependencyOrder::None);
        assert!(
            matches!(resolve(&[item("server", "1", &client)]), Err(Error::Missing { owner, target, .. }) if owner == "server" && target == "client")
        );
    }

    #[test]
    fn detailed_dependency_and_rule_defaults_apply_to_dedicated_installations() {
        let source = manifest(
            "owner",
            "[dependencies]\nbase={version='^1'}\n[incompatible]\nbad={version='*'}",
        );
        let declared = from_manifest(&source.dependencies, Side::Server);
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0].side, Side::Both);
        let active = rules(&source, Side::Server);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "bad");
    }

    #[test]
    fn long_policy_reasons_are_preserved_for_dependencies_and_pair_rules() {
        let reason = "Some context. ".repeat(100);
        let entry = format!("target={{version='^1',reason='{reason}'}}");
        let source = manifest(
            "owner",
            &format!("[dependencies]\n{entry}\n[incompatible]\n{entry}\n[discouraged]\n{entry}"),
        );
        let dependency = from_manifest(&source.dependencies, Side::Client);
        assert_eq!(dependency[0].reason.as_deref(), Some(reason.as_str()));
        let pairs = rules(&source, Side::Client);
        assert_eq!(pairs.len(), 2);
        for rule in pairs {
            assert_eq!(rule.reason.as_deref(), Some(reason.as_str()));
        }
    }

    #[test]
    fn policy_schema_errors_are_refused_when_the_manifest_is_parsed() {
        let assert_location = |result: Result<crate::Manifest, String>, line, column| {
            let error = result.unwrap_err();
            assert!(
                error.contains(&format!(" (line {line}, column {column})")),
                "{error}"
            );
        };
        for table in ["dependencies", "incompatible", "discouraged"] {
            let manifest = |entry: &str| {
                crate::Manifest::parse(&format!(
                    "id='owner'\nname='Owner'\nversion='1'\n[{table}]\n{entry}"
                ))
            };
            assert_location(
                manifest("target={version='^1',url='http://example.test'}"),
                5,
                26,
            );
            assert_location(
                crate::Manifest::parse(&format!(
                    "id='owner'\nname='Owner'\nversion='1'\n[{table}.target]\nversion='^1'\nreason='Why'\nurl='http://example.test'\n"
                )),
                7,
                5,
            );
            assert_location(manifest("target={version='^1',reason=42}"), 5, 29);
            assert_location(manifest("'bad id'='^1'"), 5, 1);
            assert!(manifest("target='not a semver requirement'").is_err());
        }
        assert_location(
            crate::Manifest::parse(
                "id='owner'\nname='Owner'\nversion='1'\nload_before = ['first', 'bad id']\n",
            ),
            4,
            25,
        );
        assert_location(
            crate::Manifest::parse(
                "id='owner'\nname='Owner'\nversion='1'\n\
                 dependencies = { first = { version = '*', url = 'https://a.test' }, second = { version = '*', url = 'http://b.test' } }\n",
            ),
            4,
            101,
        );
        assert_location(
            crate::Manifest::parse(
                "id='owner'\nname='Owner'\nversion='1'\n[dependencies]\ntarget={optional=true}\n",
            ),
            5,
            8,
        );
        assert_location(
            crate::Manifest::parse(
                "id='owner'\nname='Owner'\nversion='1'\n[dependencies.target]\noptional=true\n",
            ),
            4,
            1,
        );
    }

    #[test]
    fn positional_policy_arrays_are_refused() {
        for (table, fields) in [
            (
                "dependencies",
                "'^1', false, 'both', 'Context', 'https://example.test', 'after'",
            ),
            (
                "incompatible",
                "'<2', 'Conflicts', 'https://example.test/old', 'client'",
            ),
            (
                "discouraged",
                "'*', 'Slow', 'https://example.test/slow', 'server'",
            ),
        ] {
            let parsed = crate::Manifest::parse(&format!(
                "id='owner'\nname='Owner'\nversion='1'\n[{table}]\ntarget = [{fields}]"
            ));
            assert!(parsed.is_err(), "{table} accepted an array");
        }
    }

    #[test]
    fn override_files_validate_ids_and_include_the_path_in_read_and_parse_errors() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(OVERRIDES);
        assert_eq!(
            Overrides::read(home.path()).unwrap().get("owner", "target"),
            None
        );
        std::fs::write(&path, "[owner]\ntarget='drop'").unwrap();
        assert_eq!(
            Overrides::read(home.path()).unwrap().get("owner", "target"),
            Some(Override::Drop)
        );
        for text in [
            "[",
            "['bad owner']\ntarget='drop'",
            "[owner]\n'bad target'='drop'",
            "[owner]\ntarget='unknown action'",
        ] {
            std::fs::write(&path, text).unwrap();
            let error = Overrides::read(home.path()).unwrap_err();
            assert!(error.contains(path.to_str().unwrap()), "{error}");
            if text.contains("bad ") {
                assert!(error.contains("invalid dependency override mod id"));
            }
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let error = Overrides::read(home.path()).unwrap_err();
        assert!(error.contains(path.to_str().unwrap()));
    }

    #[test]
    fn content_dependencies_validate_and_order_without_native_api_packages() {
        let manifest = |id: &str, version: &str, dependencies: &str| {
            crate::Manifest::parse(&format!(
                "id='{id}'\nname='{id}'\nversion='{version}'\napi='0.2'\ncapabilities=['registries']\n{dependencies}"
            )).unwrap()
        };
        let addon = manifest(
            "addon",
            "1",
            "[dependencies]\nbase='^1'\nextra={version='^2', optional=true}",
        );
        let base = manifest("base", "1.2", "");
        let dependencies = from_manifest(&addon.dependencies, crate::manifest::Side::Both);
        let mods = [
            Mod {
                id: &addon.id,
                version: &addon.version,
                code: false,
                dependencies: &dependencies,
            },
            Mod {
                id: &base.id,
                version: &base.version,
                code: false,
                dependencies: &[],
            },
        ];
        assert_eq!(resolve(&mods).unwrap().order, [1, 0]);
        assert!(matches!(resolve(&mods[..1]), Err(Error::Missing { .. })));
        let incompatible = Mod {
            id: "base",
            version: &Version::new(2, 0, 0),
            code: false,
            dependencies: &[],
        };
        assert!(matches!(
            resolve(&[mods[0].clone(), incompatible]),
            Err(Error::Version { .. })
        ));
        let back = from_manifest(
            &manifest("base", "1", "[dependencies]\naddon='^1'").dependencies,
            crate::manifest::Side::Both,
        );
        assert!(matches!(
            resolve(&[
                mods[0].clone(),
                Mod {
                    id: "base",
                    version: &Version::new(1, 0, 0),
                    code: false,
                    dependencies: &back
                }
            ]),
            Err(Error::Cycle { .. })
        ));
    }

    #[test]
    fn load_rules_refuse_warn_order_and_yield_to_overrides() {
        let header =
            "id='user'\nname='User'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n";
        let user = crate::Manifest::parse(&format!(
            "{header}load_before=['late', 'absent']\n[incompatible]\n\
             broken={{version='<2', reason='Corrupts saves', url='https://example.com/why'}}\n\
             client_only={{side='client'}}\n[discouraged]\nslow='*'\n"
        ))
        .unwrap();
        let server = rules(&user, Side::Server);
        let client = rules(&user, Side::Client);
        let none = Overrides::default();
        let with = |others: &[(&'static str, &'static str)], rules: &[Rule], overrides| {
            let mut mods = vec![item("user", "1", &[])];
            mods.extend(others.iter().map(|&(id, version)| item(id, version, &[])));
            let rules = [rules.to_vec()];
            resolve_with_rules(&mods, &rules, overrides)
        };
        let refused = with(&[("broken", "1.5")], &server, &none).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "user is incompatible with broken 1.5.0 (Corrupts saves https://example.com/why)"
        );
        assert!(with(&[("broken", "2.0")], &server, &none).is_ok());
        // A client-only rule holds nothing on a dedicated server.
        assert!(with(&[("client_only", "1")], &server, &none).is_ok());
        assert!(with(&[("client_only", "1")], &client, &none).is_err());
        let slow = with(&[("slow", "3")], &server, &none).unwrap();
        assert_eq!(
            slow.warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["user works badly with slow 3.0.0"]
        );
        // Before a present mod it doesn't depend on; an absent one is ignored.
        let mods = [item("late", "1", &[]), item("user", "1", &[])];
        let rules = [Vec::new(), server.clone()];
        assert_eq!(
            resolve_with_rules(&mods, &rules, &none).unwrap().order,
            [1, 0]
        );
        let discourage: Overrides = toml::from_str("[user]\nbroken = 'discourage'\n").unwrap();
        let warned = with(&[("broken", "1.5")], &server, &discourage).unwrap();
        assert_eq!(
            warned
                .warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            [
                "dependency override: user -> broken only discouraged",
                "user works badly with broken 1.5.0 (Corrupts saves https://example.com/why)"
            ]
        );
        let drop: Overrides = toml::from_str("[user]\nbroken = 'drop'\n").unwrap();
        let dropped = with(&[("broken", "1.5")], &server, &drop).unwrap();
        assert_eq!(
            dropped
                .warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["dependency override: user -> broken dropped"]
        );
        for refused in [
            "[incompatible]\nx={url='http://example.com'}",
            "load_before=['Bad']",
        ] {
            assert!(crate::Manifest::parse(&format!("{header}{refused}")).is_err());
        }
    }

    #[test]
    fn dispatch_order_and_validation_overrides_never_replace_the_link_graph() {
        for order in ["none", "before"] {
            let declared = deps(&format!(
                "[dependencies]\nbase={{version='^1',order='{order}'}}"
            ));
            let mods = [item("user", "1", &declared), item("base", "1", &[])];
            assert_eq!(resolve(&mods).unwrap().order, [0, 1]);
            assert_eq!(resolve_links(&mods).unwrap().order, [1, 0]);
        }
        let declared = deps(
            "[dependencies]\nbase={version='^2',reason='Integration',url='https://example.test/base'}",
        );
        let mods = [item("user", "1", &declared), item("base", "1", &[])];
        assert!(matches!(resolve(&mods), Err(Error::Version { .. })));
        let dropped: Overrides = toml::from_str("[user]\nbase='drop'").unwrap();
        let logical = resolve_with_rules(&mods, &[], &dropped).unwrap();
        assert_eq!(logical.order, [0, 1]);
        assert_eq!(logical.warnings[0].kind, NoticeKind::OverrideDropped);
        assert_eq!(logical.warnings[0].reason.as_deref(), Some("Integration"));
        assert_eq!(
            logical.warnings[0].url.as_deref(),
            Some("https://example.test/base")
        );
        assert_eq!(resolve_links(&mods).unwrap().order, [1, 0]);
        assert_eq!(
            resolve_with_rules(&mods[..1], &[], &dropped).unwrap().order,
            [0]
        );
        assert!(
            matches!(resolve_links(&mods[..1]), Err(Error::Missing { owner, target, .. }) if owner == "user" && target == "base")
        );
    }

    #[test]
    fn only_supplied_native_packages_contribute_link_edges() {
        let required = deps("[dependencies]\nlibrary='^1'\ndata='^1'");
        let mods = [
            item("consumer", "1", &required),
            item("library", "1", &[]),
            Mod {
                id: "data",
                version: &Version::new(1, 0, 0),
                code: false,
                dependencies: &[],
            },
        ];
        assert_eq!(resolve_links(&mods).unwrap().order, [1, 0, 2]);
        assert_eq!(resolve(&mods).unwrap().order, [1, 2, 0]);
    }

    fn manifest(id: &str, extra: &str) -> crate::Manifest {
        crate::Manifest::parse(&format!(
            "id='{id}'\nname='{id}'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n{extra}"
        ))
        .unwrap()
    }

    #[test]
    fn content_and_mixed_rules_share_refusal_without_turning_content_into_a_library() {
        let owner = manifest(
            "owner",
            "[incompatible]\nother={version='*',reason='Bad interaction',url='https://example.test/why'}",
        );
        let plain = manifest("other", "");
        let code = manifest(
            "other",
            &format!(
                "[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'",
                "a".repeat(40),
                "b".repeat(40)
            ),
        );
        for other in [&plain, &code] {
            let error = resolve_manifests(&[&owner, other], Side::Client, &Overrides::default())
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "owner is incompatible with other 1.0.0 (Bad interaction https://example.test/why)"
            );
        }
        let dependent = manifest("dependent", "[dependencies]\nother='^1'");
        assert_eq!(
            resolve_manifests(&[&dependent, &code], Side::Client, &Overrides::default())
                .unwrap()
                .order,
            [1, 0]
        );
    }

    #[test]
    fn required_content_is_deferred_by_build_but_required_by_installation() {
        let content = manifest("data", "");
        let native = format!(
            "[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'",
            "a".repeat(40),
            "b".repeat(40)
        );
        let dependent = manifest("behaviour", &format!("[dependencies]\ndata='^1'\n{native}"));
        let build = build_dependencies(&dependent, Side::Client, |_| false);
        let code = Mod {
            id: &dependent.id,
            version: &dependent.version,
            code: true,
            dependencies: &build,
        };
        assert_eq!(resolve_links(&[code]).unwrap().order, [0]);
        assert_eq!(
            resolve_manifests(&[&dependent], Side::Client, &Overrides::default())
                .unwrap_err()
                .to_string(),
            "behaviour requires missing mod data"
        );
        let resolved =
            resolve_manifests(&[&dependent, &content], Side::Client, &Overrides::default())
                .unwrap();
        assert_eq!(resolved.order, [1, 0]);
        assert!(resolved.effective.permits("behaviour", "data"));
    }

    #[test]
    fn permission_and_dispatch_use_the_same_filtered_rows() {
        use mod_vocab::DependencyKind;
        let dependent = manifest(
            "addon",
            "[dependencies]\nowner={version='^1',order='before'}\nabsent={version='*',optional=true}\nserver={version='^1',side='server'}\nnone={version='^1',order='none'}\n",
        );
        let owner = manifest("owner", "");
        let server = manifest("server", "");
        let none = manifest("none", "load_before=['addon']");
        let inputs = [&owner, &server, &dependent, &none];
        let resolution = resolve_manifests(&inputs, Side::Client, &Overrides::default()).unwrap();
        assert_eq!(resolution.order, [1, 3, 2, 0]);
        assert_eq!(
            resolution.effective.kind("addon", "owner"),
            Some(DependencyKind::Required)
        );
        assert_eq!(
            resolution.effective.kind("addon", "none"),
            Some(DependencyKind::Required)
        );
        assert_eq!(
            resolution.effective.kind("addon", "absent"),
            Some(DependencyKind::Optional)
        );
        assert!(!resolution.effective.permits("addon", "server"));
        assert!(!resolution.effective.permits("owner", "addon"));
        assert!(!resolution.effective.permits("none", "addon"));
        for action in ["drop", "discourage"] {
            let overrides =
                toml::from_str(&format!("[addon]\nowner='{action}'\nabsent='{action}'")).unwrap();
            let resolution = resolve_manifests(&inputs, Side::Client, &overrides).unwrap();
            assert_eq!(resolution.order, [0, 1, 3, 2]);
            assert!(!resolution.effective.permits("addon", "owner"));
            assert_eq!(resolution.effective.kind("addon", "absent"), None);
        }
    }

    #[test]
    fn a_content_cycle_reports_its_literal_path_without_unrelated_nodes() {
        let a = manifest("a", "[dependencies]\nb='^1'");
        let b = manifest("b", "[dependencies]\na='^1'");
        let free = manifest("free", "");
        assert_eq!(
            resolve_manifests(&[&a, &b, &free], Side::Client, &Overrides::default())
                .unwrap_err()
                .to_string(),
            "Mod dependency cycle: a -> b -> a"
        );
        let dropped: Overrides = toml::from_str("[a]\nb='drop'").unwrap();
        assert_eq!(
            resolve_manifests(&[&a, &b, &free], Side::Client, &dropped)
                .unwrap()
                .order,
            [0, 1, 2]
        );
    }

    #[test]
    fn order_and_ties_include_present_optional() {
        let user = deps("[dependencies]\nbase = '^1'\nextra = { version = '^2', optional = true }");
        let mods = [
            item("user", "1.0.0", &user),
            item("base", "1.2", &[]),
            item("extra", "2", &[]),
            item("other", "1.0.0", &[]),
        ];
        let result = resolve(&mods).unwrap();
        assert_eq!(result.order, [1, 2, 0, 3]);
        assert_eq!(
            resolve(&[item("user", "1.0.0", &user), item("base", "1.2.0", &[])])
                .unwrap()
                .order,
            [1, 0]
        );
    }
    #[test]
    fn failures_name_mods() {
        let required = deps("[dependencies]\nbase = '^1'");
        let optional = deps("[dependencies]\nbase = { version = '^1', optional = true }");
        for dependencies in [&required, &optional] {
            let err = resolve(&[
                item("user", "1.0.0", dependencies),
                item("base", "2.0.0", &[]),
            ])
            .err()
            .unwrap()
            .to_string();
            assert!(err.contains("user") && err.contains("base"));
        }
        let err = resolve(&[item("user", "1.0.0", &required)])
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("user") && err.contains("base"));
        let back = deps("[dependencies]\nuser = '^1'");
        let err = resolve(&[
            item("user", "1.0.0", &required),
            item("base", "1.0.0", &back),
        ])
        .err()
        .unwrap()
        .to_string();
        assert!(err.contains("user") && err.contains("base"));
    }
    #[test]
    fn rewrites_installed_and_absent_optional_dependencies() {
        let deps = deps("[dependencies]\nbase = '^1'\nextra = { version = '^2', optional = true }");
        let mut manifest: toml::Table = "[dependencies]\nsunrise-mod-api = '1'".parse().unwrap();
        let packages = [
            Package {
                dependency: &deps[0],
                package: Some("base-package"),
            },
            Package {
                dependency: &deps[1],
                package: Some("extra-package"),
            },
        ];
        assert_eq!(
            rewrite("user", &mut manifest, &packages).unwrap(),
            ["extra"]
        );
        assert_eq!(
            manifest["dependencies"]["base"]["path"].as_str(),
            Some("../base")
        );
        assert_eq!(
            manifest["dependencies"]["base"]["package"].as_str(),
            Some("base-package")
        );
        assert_eq!(
            manifest["dependencies"]["extra"]["optional"].as_bool(),
            Some(true)
        );
        assert_eq!(manifest["features"]["extra"][0].as_str(), Some("dep:extra"));
        let mut absent = toml::Table::new();
        assert!(
            rewrite(
                "user",
                &mut absent,
                &[Package {
                    dependency: &deps[1],
                    package: None
                }]
            )
            .unwrap()
            .is_empty()
        );
        assert!(absent["features"]["extra"].as_array().unwrap().is_empty());
        assert!(absent["dependencies"].get("extra").is_none());
    }

    #[test]
    fn rejects_conflicting_features_and_dependency_names_atomically() {
        let deps = deps("[dependencies]\nextra = { version = '^2', optional = true }");
        for text in [
            "[features]\ndefault = ['extra']",
            "[dependencies]\nextra = '1'",
        ] {
            let mut manifest: toml::Table = text.parse().unwrap();
            let original = manifest.clone();
            assert!(
                rewrite(
                    "user",
                    &mut manifest,
                    &[Package {
                        dependency: &deps[0],
                        package: Some("extra-package")
                    }]
                )
                .is_err()
            );
            assert_eq!(manifest, original);
        }
    }

    #[test]
    fn malformed_dependency_is_refused() {
        let value: toml::Table = "[dependencies]\nbase = { version = '^1', optional = 'yes' }"
            .parse()
            .unwrap();
        assert!(parse("user", value.get("dependencies")).is_err());
    }

    #[test]
    fn served_by_validation_orders_after_host_enforces_sides_and_ignores_overrides() {
        let native = |id: &str, side: &str, serves: bool, extra: &str| {
            manifest(
                id,
                &format!(
                    "side='{side}'\n{extra}\n[native]\ngame='26.3'\nsunrise='{}'\nserves_content={serves}\n",
                    "a".repeat(40),
                ),
            )
        };
        let content = |id: &str, side: &str, host: &str| {
            manifest(id, &format!("side='{side}'\nserved_by='{host}'\n"))
        };

        let runes = content("runes", "both", "horizon_java");
        let missing = validate_served_by(&[&runes]).unwrap_err();
        assert!(
            matches!(
                &missing,
                Error::InvalidServer {
                    owner,
                    target,
                    reason: InvalidServerReason::MissingHost,
                } if owner == "runes" && target == "horizon_java"
            ),
            "{missing}"
        );
        // dependency-overrides.toml cannot drop served_by validation.
        let drop_override: Overrides = toml::from_str("[runes]\nhorizon_java='drop'\n").unwrap();
        assert!(matches!(
            resolve_manifests(&[&runes], Side::Client, &drop_override),
            Err(Error::InvalidServer {
                owner,
                target,
                reason: InvalidServerReason::MissingHost,
            }) if owner == "runes" && target == "horizon_java"
        ));

        let content_target = manifest("horizon_java", "side='both'\n");
        let err = validate_served_by(&[&runes, &content_target]).unwrap_err();
        assert!(
            matches!(
                &err,
                Error::InvalidServer {
                    owner,
                    target,
                    reason: InvalidServerReason::HostIsContentPackage,
                } if owner == "runes" && target == "horizon_java"
            ),
            "{err}"
        );

        let unopted = native("horizon_java", "both", false, "");
        let err = validate_served_by(&[&runes, &unopted]).unwrap_err();
        assert!(
            matches!(
                &err,
                Error::InvalidServer {
                    owner,
                    target,
                    reason: InvalidServerReason::HostDoesNotServeContent,
                } if owner == "runes" && target == "horizon_java"
            ),
            "{err}"
        );

        for ((host_side, expected_host), (content_side, expected_content)) in [
            (("client", Side::Client), ("both", Side::Both)),
            (("server", Side::Server), ("both", Side::Both)),
            (("server", Side::Server), ("client", Side::Client)),
            (("client", Side::Client), ("server", Side::Server)),
        ] {
            let host = native("horizon_java", host_side, true, "");
            let pkg = content("runes", content_side, "horizon_java");
            let err = validate_served_by(&[&pkg, &host]).unwrap_err();
            assert!(
                matches!(
                    &err,
                    Error::InvalidServer {
                        owner,
                        target,
                        reason: InvalidServerReason::SideNotCovered { host, content },
                    } if owner == "runes"
                        && target == "horizon_java"
                        && *host == expected_host
                        && *content == expected_content
                ),
                "host={host_side} content={content_side}: {err}"
            );
        }

        for (host_side, content_side) in [
            ("both", "both"),
            ("both", "client"),
            ("client", "client"),
            ("both", "server"),
            ("server", "server"),
        ] {
            let host = native("horizon_java", host_side, true, "");
            let pkg = content("runes", content_side, "horizon_java");
            assert!(
                validate_served_by(&[&pkg, &host]).is_ok(),
                "host={host_side} content={content_side}"
            );
        }

        // Content is ordered after its host, even when dependency-overrides.toml tries to drop it.
        let host = native("horizon_java", "both", true, "");
        let resolved = resolve_manifests(&[&runes, &host], Side::Client, &drop_override).unwrap();
        assert_eq!(resolved.order, [1, 0]);

        // A host depending on its served content creates a cycle.
        let cyclic_host = native("horizon_java", "both", true, "[dependencies]\nrunes='^1'");
        assert!(matches!(
            resolve_manifests(&[&runes, &cyclic_host], Side::Client, &Overrides::default()),
            Err(Error::Cycle { .. })
        ));
    }
}
